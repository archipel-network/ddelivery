use std::{net::TcpListener, sync::mpsc::Sender};

use log::{debug, error, info};

use crate::{mail_sender::SenderMsg, smtp::Session};

pub struct SmtpConfig {
    pub bind: String
}

pub fn run_smtp_server(config: SmtpConfig, mail_sender_channel: Sender<SenderMsg>) {
    debug!("Starting SMTP server task");

    let listener = TcpListener::bind(config.bind.clone())
        .expect("Failed to bind SMTP socket");

    info!("SMTP listening on {}", config.bind);

    for incoming in listener.incoming()
        .filter_map(|r| r.inspect_err(|e| error!("Failed to accept SMTP connection : {e}")).ok()) {

        debug!("Connection started");

        let session = Session::new(incoming, "ddelivery".to_owned())
            .unwrap();

        let Ok(mail_iter) = session.into_mail_iter() else {
            return;
        };

        for mail in mail_iter {
            //TODO Make mail sending fail if bundle submission failed
            match mail {
                Ok(mail) => {
                    debug!("Received email from {:?} to {:?}", mail.from, mail.receipients);
                    if let Err(e) = mail_sender_channel.send(SenderMsg::SendMail(mail)){
                        error!("Failed to send mail to sender task: {e}")
                    }
                },
                Err(e) => error!("Failed to receive mail : {e}")
            }
        }

        debug!("Connection ended")
    }
}
