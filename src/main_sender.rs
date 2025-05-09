mod smtp_server;
mod smtp;
mod mail_sender;
mod defaults;

use std::{env, path::Path, sync::mpsc, thread};

use defaults::OUTBOX_AGENT_ID;
use log::info;
use mail_sender::run_sender_task;
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
        OUTBOX_AGENT_ID.to_owned()
    ).expect("Failed to connect to archipel-core");

    info!("Outbox connected to archipel-core {}{}", outbox_agent.node_eid, outbox_agent.agent_id);

    let (sender, receiver) = mpsc::channel::<mail_sender::SenderMsg>();

    thread::scope(|s| {
        s.spawn(|| {
            run_sender_task(receiver, outbox_agent)
        });

        run_smtp_server(SmtpConfig {
            bind: "127.0.0.1:2525".to_owned()
        }, sender.clone());

        sender.send(mail_sender::SenderMsg::ShutdownTask)
            .expect("Failed to send shutdown message");
    });

}