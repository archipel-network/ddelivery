mod smtp_server;
mod smtp;
use simple_logger::SimpleLogger;
use smtp_server::{run_smtp_server, SmtpConfig};

fn main() {
    SimpleLogger::new().init()
        .expect("Failed to start log system");

    let zmq = zmq::Context::new();

    run_smtp_server(SmtpConfig {
        bind: "127.0.0.1:2525".to_owned()
    }, zmq)
}