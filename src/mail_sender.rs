use std::sync::mpsc::Receiver;

use log::debug;

use crate::{defaults::INBOX_AGENT_ID, smtp::Mail};

pub enum SenderMsg {
    SendMail(Mail),
    ShutdownTask
}

pub fn run_sender_task(receiver: Receiver<SenderMsg>, mut outbox_agent: ud3tn_aap::Agent){
    debug!("Starting mail sender task");

    for msg in receiver {
        match msg {
            SenderMsg::ShutdownTask => break,
            SenderMsg::SendMail(mail) => {
                for recipient in mail.receipients.into_iter() {
                    let detination = format!("dtn://{}/{}", recipient.domain(), INBOX_AGENT_ID);
                    debug!("Sending mail to {detination}");

                    outbox_agent.send_bundle(
                        detination,
                        &mail.content
                    ).expect("Failed to send mail to node");
                }
            },
        }
    }
}