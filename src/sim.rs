//! Драйвер симуляции: гоним поток операций через настоящую книгу и пишем результат
//! в CSV для офлайн-анализа (pandas).
//!
//! Пишем два файла:
//! - `trades.csv` — по строке на каждую состоявшуюся сделку (ценовой/объёмный поток);
//! - `book.csv`   — снимок верха книги после каждой операции (best bid/ask, спред,
//!   глубина, mid, сколько наторговали на этом шаге).
//!
//! CSV пишется руками: все колонки числовые либо короткие ASCII-метки, экранирование
//! не нужно, а зависимостей по-прежнему ноль — сборка остаётся мгновенной. pandas
//! читает такой CSV без настройки (`pd.read_csv`).

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::events::{Event, RejectReason};
use crate::generator::{Generator, GeneratorConfig, OpKind};
use crate::ops::Op;
use crate::orderbook::OrderBook;
use crate::types::{Price, Side};

#[derive(Debug, Clone, Copy)]
pub struct SimConfig {
    pub gen_cfg: GeneratorConfig,
    pub seed: u64,
    /// Сколько операций прогнать.
    pub steps: usize,
    /// Ёмкость пула ордеров. Должна покрывать пик одновременно живых заявок.
    pub capacity: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            gen_cfg: GeneratorConfig::default(),
            seed: 0xC0FFEE,
            steps: 1_000_000,
            capacity: 100_000,
        }
    }
}

/// Сводка прогона — печатается в конце, чтобы сразу видеть «жив ли» поток.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimStats {
    pub steps: usize,
    pub trades: u64,
    pub traded_qty: i64,
    pub accepted: u64,
    pub cancelled: u64,
    pub rejected: u64,
    /// Отмена несуществующего/исполненного ордера — норма (поток не знает, что
    /// его пассивный ордер уже съели матчингом).
    pub rejected_not_found: u64,
    /// Цена вышла за фиксированный ладдер — это уже сигнал, что коридор генератора
    /// шире книги (конфиг надо чинить), а не нормальное явление.
    pub rejected_out_of_range: u64,
}

fn side_tag(side: Side) -> &'static str {
    match side {
        Side::Bid => "bid",
        Side::Ask => "ask",
    }
}

/// Записывает цену, либо пустое поле (→ NaN в pandas), если стороны книги нет.
fn write_price_opt<W: Write>(w: &mut W, p: Option<Price>) -> io::Result<()> {
    match p {
        Some(p) => write!(w, "{}", p.raw()),
        None => Ok(()),
    }
}

pub fn run<P: AsRef<Path>>(cfg: SimConfig, trades_path: P, book_path: P) -> io::Result<SimStats> {
    let reference = Price::new(cfg.gen_cfg.start_mid).expect("start_mid must be > 0");
    let mut book = OrderBook::new(reference, cfg.capacity);
    let mut generator = Generator::new(cfg.gen_cfg, cfg.seed);

    let mut trades_w = BufWriter::new(File::create(trades_path)?);
    let mut book_w = BufWriter::new(File::create(book_path)?);
    writeln!(trades_w, "seq,ts,price,qty,aggressor,maker_id,taker_id")?;
    writeln!(
        book_w,
        "seq,ts,op_kind,best_bid,best_ask,spread,mid,bid_qty,ask_qty,step_trades,step_qty"
    )?;

    let mut stats = SimStats::default();
    let mut events: Vec<Event> = Vec::new();

    for seq in 0..cfg.steps {
        let ts = seq as u64; // логическое время = номер события
        // Верх книги ДО операции — генератор держит на нём mid на поводке.
        let pre_bb = book.best_bid().map(|p| p.raw());
        let pre_ba = book.best_ask().map(|p| p.raw());
        let (op, kind) = generator.next_op(pre_bb, pre_ba);

        // Сторона тейкера для разметки сделок — это сторона текущего Place.
        let taker_side = match &op {
            Op::Place { side, .. } => Some(*side),
            Op::Cancel { .. } => None,
        };

        events.clear();
        book.apply(op, &mut events);

        // 1. Сделки этого шага → trades.csv + агрегаты для book.csv.
        let mut step_trades = 0u64;
        let mut step_qty = 0i64;
        for e in &events {
            match e {
                Event::Trade {
                    maker_order_id,
                    taker_order_id,
                    price,
                    qty,
                    ..
                } => {
                    let aggressor = taker_side.map(side_tag).unwrap_or("");
                    writeln!(
                        trades_w,
                        "{seq},{ts},{},{},{aggressor},{},{}",
                        price.raw(),
                        qty.raw(),
                        maker_order_id.0,
                        taker_order_id.0,
                    )?;
                    step_trades += 1;
                    step_qty += qty.raw();
                    stats.trades += 1;
                    stats.traded_qty += qty.raw();
                }
                Event::Accepted { .. } => stats.accepted += 1,
                Event::Cancelled { .. } => stats.cancelled += 1,
                Event::Rejected { reason, .. } => {
                    stats.rejected += 1;
                    match reason {
                        RejectReason::OrderNotFound => stats.rejected_not_found += 1,
                        RejectReason::PriceOutOfRange => stats.rejected_out_of_range += 1,
                        _ => {}
                    }
                }
            }
        }

        // 2. Снимок верха книги после операции.
        let bb = book.best_bid();
        let ba = book.best_ask();
        let bid_qty = book.best_bid_qty();
        let ask_qty = book.best_ask_qty();

        write!(book_w, "{seq},{ts},{},", op_kind_tag(kind))?;
        write_price_opt(&mut book_w, bb)?;
        write!(book_w, ",")?;
        write_price_opt(&mut book_w, ba)?;
        write!(book_w, ",")?;
        // спред и mid — только когда есть обе стороны
        match (bb, ba) {
            (Some(b), Some(a)) => {
                let spread = a.raw() - b.raw();
                // mid как f64 (полутики), удобно для анализа ценового ряда
                let mid = (a.raw() + b.raw()) as f64 / 2.0;
                write!(book_w, "{spread},{mid}")?;
            }
            _ => write!(book_w, ",")?, // обе пустые → NaN,NaN
        }
        writeln!(book_w, ",{bid_qty},{ask_qty},{step_trades},{step_qty}")?;
    }

    trades_w.flush()?;
    book_w.flush()?;

    stats.steps = cfg.steps;
    Ok(stats)
}

fn op_kind_tag(kind: OpKind) -> &'static str {
    match kind {
        OpKind::PassivePlace => "passive",
        OpKind::AggressivePlace => "aggressive",
        OpKind::Cancel => "cancel",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_produces_trades_and_book_files() {
        let dir = std::env::temp_dir();
        let trades = dir.join("ob_sim_trades_test.csv");
        let book = dir.join("ob_sim_book_test.csv");

        let cfg = SimConfig {
            steps: 5_000,
            ..SimConfig::default()
        };
        let stats = run(cfg, &trades, &book).unwrap();

        assert_eq!(stats.steps, 5_000);
        // на 5k шагов с дефолтным конфигом сделки обязаны случиться
        assert!(stats.trades > 0, "ожидали хоть какие-то сделки");
        assert!(stats.accepted > 0);

        // book.csv: заголовок + ровно steps строк
        let text = std::fs::read_to_string(&book).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next().unwrap(),
            "seq,ts,op_kind,best_bid,best_ask,spread,mid,bid_qty,ask_qty,step_trades,step_qty"
        );
        assert_eq!(lines.count(), 5_000);

        let _ = std::fs::remove_file(&trades);
        let _ = std::fs::remove_file(&book);
    }

    #[test]
    fn book_tracks_latent_price_tight_spread() {
        // Регресс: без «поводка» книга отставала от latent mid на сотни тиков и спред
        // раздувался до ~335. С поводком стакан остаётся тугим, а навал стоячей
        // ликвидности на одном уровне не разрастается.
        let cfg = SimConfig::default();
        let reference = Price::new(cfg.gen_cfg.start_mid).unwrap();
        let mut book = OrderBook::new(reference, cfg.capacity);
        let mut g = Generator::new(cfg.gen_cfg, cfg.seed);
        let mut events = Vec::new();

        let mut warm = 0; // прогрев, пока обе стороны не появятся
        let mut spread_sum = 0i64;
        let mut spread_max = 0i64;
        let mut samples = 0i64;

        for _ in 0..80_000 {
            let bb = book.best_bid().map(|p| p.raw());
            let ba = book.best_ask().map(|p| p.raw());
            let (op, _) = g.next_op(bb, ba);
            events.clear();
            book.apply(op, &mut events);

            if let (Some(b), Some(a)) = (book.best_bid(), book.best_ask()) {
                warm += 1;
                if warm > 2_000 {
                    let s = a.raw() - b.raw();
                    spread_sum += s;
                    spread_max = spread_max.max(s);
                    samples += 1;
                }
            }
        }

        let avg = spread_sum as f64 / samples as f64;
        assert!(
            avg < 15.0,
            "средний спред должен быть тугим, получили {avg:.1}"
        );
        assert!(
            spread_max < 80,
            "макс. спред не должен взрываться, получили {spread_max}"
        );

        // и книга не отстаёт от latent mid больше, чем на поводок + глубину
        let book_mid = {
            let b = book.best_bid().unwrap().raw();
            let a = book.best_ask().unwrap().raw();
            (a + b) / 2
        };
        let lag = (book_mid - g.mid()).abs();
        assert!(
            lag < cfg.gen_cfg.leash + cfg.gen_cfg.passive_depth + 5,
            "книга отстала на {lag}"
        );
    }

    #[test]
    fn no_rejected_under_default_config() {
        // Дефолтный коридор должен укладываться в ладдер: цены не отвергаются по
        // диапазону, пул не переполняется.
        let dir = std::env::temp_dir();
        let trades = dir.join("ob_sim_trades_rej.csv");
        let book = dir.join("ob_sim_book_rej.csv");

        let stats = run(
            SimConfig {
                steps: 50_000,
                ..Default::default()
            },
            &trades,
            &book,
        )
        .unwrap();
        assert_eq!(
            stats.rejected_out_of_range, 0,
            "коридор генератора должен укладываться в ладдер"
        );

        let _ = std::fs::remove_file(&trades);
        let _ = std::fs::remove_file(&book);
    }
}
