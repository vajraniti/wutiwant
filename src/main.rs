mod events;
mod generator;
mod ladder;
mod level;
mod ops;
mod order;
mod orderbook;
mod pool;
mod rng;
mod sim;
mod types;

use sim::SimConfig;

fn main() {
    let cfg = SimConfig::default();
    println!(
        "running {} steps (seed {:#x}, capacity {})...",
        cfg.steps, cfg.seed, cfg.capacity
    );

    let started = std::time::Instant::now();
    let stats = match sim::run(cfg, "trades.csv", "book.csv") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("simulation failed: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();

    println!("done in {:.3?}", elapsed);
    println!("  steps     : {}", stats.steps);
    println!("  accepted  : {}", stats.accepted);
    println!("  trades    : {}", stats.trades);
    println!("  traded qty: {}", stats.traded_qty);
    println!("  cancelled : {}", stats.cancelled);
    println!(
        "  rejected  : {} (not_found {}, out_of_range {})",
        stats.rejected, stats.rejected_not_found, stats.rejected_out_of_range
    );
    println!("wrote trades.csv and book.csv");
}
