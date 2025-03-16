use std::net::TcpListener;

use log::{debug, error, info};

use crate::smtp::Session;

pub struct SmtpConfig {
    pub bind: String
}

pub fn run_smtp_server(config: SmtpConfig, _zmq: zmq::Context) {
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
            match mail {
                Ok(mail) => {
                    debug!("Received email from {:?} to {:?}", mail.from, mail.receipients);
                    debug!("\n{}", String::from_utf8_lossy(&mail.content))
                },
                Err(e) => error!("Failed to receive mail : {e}")
            }
        }

        debug!("Connection ended")
    }
}
