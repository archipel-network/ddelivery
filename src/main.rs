mod smtp_server;
mod smtp;
use std::{env, path::Path};

use log::info;
use simple_logger::SimpleLogger;
use smtp_server::{run_smtp_server, SmtpConfig};

fn main() {
    SimpleLogger::new().init()
        .expect("Failed to start log system");

    let outbox_agent = ud3tn_aap::Agent::connect_unix(
        Path::new(
            env::var("ARCHIPEL_CORE_AAP_SOCKET")
            .unwrap_or("/run/archipel-core/archipel-core.socket".to_owned())
            .as_str()
        ),
        "mail/outbox".to_owned()
    ).expect("Failed to connect to archipel-core");

    info!("Outbox connected to archipel-core {}{}", outbox_agent.node_eid, outbox_agent.agent_id);

    let zmq = zmq::Context::new();

    run_smtp_server(SmtpConfig {
        bind: "127.0.0.1:2525".to_owned()
    }, zmq)
}