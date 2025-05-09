mod defaults;

use std::{env, path::Path};

use defaults::INBOX_AGENT_ID;
use mail_parser::MessageParser;
use mail_send::{SmtpClient, SmtpClientBuilder};
use simple_logger::SimpleLogger;
use log::{debug, error, warn};
use tokio::{io::{AsyncRead, AsyncWrite}, sync::mpsc::{UnboundedReceiver, UnboundedSender}};
use ud3tn_aap::Agent;

struct ReceivedMessage {
    raw_message: Vec<u8>,
    recipient_users: Vec<String>,
    from: String
}

#[tokio::main]
async fn main() {
    SimpleLogger::new().init()
        .expect("Failed to start log system");

    let inbox_agent = ud3tn_aap::Agent::connect_unix(
        Path::new(
            env::var("ARCHIPEL_CORE_AAP_SOCKET")
            .unwrap_or("/run/archipel-core/archipel-core.socket".to_owned())
            .as_str()
        ),
        INBOX_AGENT_ID.to_owned()
    ).expect("Failed to connect to archipel-core");

    let sender = SmtpClientBuilder::new("localhost", 24)
        .lmtp(true)
        .connect_plain()
        .await.expect("Failed to connect to LMTP server");

    let (inproc_sender, inproc_receiver) = 
        tokio::sync::mpsc::unbounded_channel::<ReceivedMessage>();
    
    let recipient_domain = inbox_agent.node_eid[6..inbox_agent.node_eid.len()-1].to_owned();

    let (_, result) = tokio::join!(
        lmtp_sender_task(sender, inproc_receiver),
        tokio::task::spawn_blocking(move || dtn_receiver_task(inbox_agent, inproc_sender, recipient_domain))
    );

    result.unwrap()
}

fn dtn_receiver_task(mut dtn_agent: Agent, inproc_sender: UnboundedSender<ReceivedMessage>, recipient_domain: String){
    
    let parser = MessageParser::default();
     
    loop {
        let (source, bundle) = match dtn_agent.recv_bundle() {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to receive mail from DTN: {e}");
                continue;
            }
        };

        let message = match parser.parse(&bundle){
            Some(m) => {
                debug!("Received mail from endpoint {source}");
                m
            },
            None => {
                error!("Invalid or empty message received from endpoint {source}");
                continue;
            }
        };

        let Some(from) = message.from()
            .and_then(|it| it.first())
            .and_then(|it| it.address.to_owned())
            .map(|it| it.to_string()) else {
                warn!("Missing from field in mail");
                continue;
        };

        let mut recipients = Vec::new();
        if let Some(to_addr) = message.to() {
            for to in to_addr.iter() {
                let Some(addr) = to.address.to_owned() else {
                    continue;
                };

                let Some((username, domain)) = addr.split_once("@") else {
                    continue;
                };

                if domain == recipient_domain {
                    recipients.push(username.to_owned());
                }
            }
        }

        drop(message);

        inproc_sender.send(ReceivedMessage {
            raw_message: bundle,
            recipient_users: recipients,
            from
        }).expect("Failed to transmit message to mail sender");
    }
}

async fn lmtp_sender_task<T: AsyncRead+AsyncWrite+Unpin>(mut sender: SmtpClient<T>, mut inproc_receiver: UnboundedReceiver<ReceivedMessage>){
   
    loop {
        let Some(source_message) = inproc_receiver.recv().await else {
            break;
        };

        if source_message.recipient_users.is_empty() {
            warn!("Received mail without local recipient");
            continue;
        }
        
        let mut message = mail_send::smtp::message::Message::empty()
        .body(source_message.raw_message)
        .from(source_message.from);

        for recipient in source_message.recipient_users {
            message = message.to(recipient);
        }

        match sender.send(message).await {
            Ok(_) => debug!("Successfully transmitted message"),
            Err(e) => error!("Failed to transmit message: {e}")
        }
    }

}