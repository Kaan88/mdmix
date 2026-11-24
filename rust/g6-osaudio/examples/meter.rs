//! Prints the G6 output peak level ten times a second for three seconds (read only):
//! `cargo run -p g6-osaudio --example meter`.
//!
//! The meter is opened on its own polling thread, which is how a GUI should use it (opening it joins the thread to
//! the COM MTA).
fn main() {
    let poller = std::thread::spawn(|| {
        let Some(meter) = g6_osaudio::open_output_meter() else {
            println!("no G6 output meter available");
            return;
        };
        for _ in 0..30 {
            println!("{:.4}", meter.peak());
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
    poller.join().expect("meter thread");
}
