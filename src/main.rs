extern crate mio_pong;

use std::thread;

use mio_pong::server::pong::Pong;
use mio_pong::client::ping::Ping;

fn start_pong() {
    let pong = Pong::start("0.0.0.0:6567".parse().ok().expect("Unable to start server"));
}

fn start_ping() {
    let ping = Ping::start("0.0.0.0:6567".parse().ok().expect("Unable to start client"));
}

fn main() {
    println!("Starting Pong!");
    let start_pong = thread::spawn(move || { start_pong() });
    println!("Starting Ping!");
    let start_ping = thread::spawn(move || { start_ping() });
}
