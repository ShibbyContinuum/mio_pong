
use Pong;
use Ping;

fn main() {
    println!("Starting Pong!");
    Pong::start("0.0.0.0:6567".parse().ok().expect("Unable to start server"));
    println!("Starting Ping!");
    Ping::start("0.0.0.0:6567".parse().ok().expect("Unable to start client"));
}
