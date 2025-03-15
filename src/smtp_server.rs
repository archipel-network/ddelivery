use std::{io::{self, Read, Write}, net::{TcpListener, TcpStream}, string::FromUtf8Error};

use log::{debug, error, info};
use thiserror::Error;

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

        debug!("Connection received");

        let mut session = Session::new(incoming, "ddelivery".to_owned())
            .unwrap();

        for command in session.recv_commands().unwrap() {
            match command {
                Ok(command) => {
                    debug!("New command {:?}", command);
                    match command {
                        ClientCommand::Hello(domain) => session.send_command(ServerCommand::HelloOk { 
                                                domain,
                                                greet: Some("delayed greetings !".to_owned())
                                            }).unwrap(),
                        ClientCommand::Mail(_) => session.send_command(ServerCommand::SenderOk).unwrap(),
                        ClientCommand::Recipient(_) => session.send_command(ServerCommand::RecipientOk).unwrap(),
                        ClientCommand::Data => session.send_command(ServerCommand::StartMailInput).unwrap(),
                        ClientCommand::MailInput(_) => session.send_command(ServerCommand::MailOk).unwrap(),
                        ClientCommand::Quit => {
                            session.send_command(ServerCommand::ClosingConnection).unwrap();
                            break;
                        },
                        _ => {}
                    }
                },
                Err(e) => error!("Failed to read commands : {e}")
            }
            
        }

        session.shutdown().unwrap();
        debug!("Connection ended")
    }
}

#[derive(Debug)]
pub struct Session {
    source: TcpStream
}

impl Session {
    pub fn new(mut source: TcpStream, domain: String) -> Result<Self, io::Error> {
        if let Err(e) = source.write_all(
            &ServerCommand::OpeningMessage(domain.clone()).into_bytes()) {
            return Err(e);
        }

        Ok(Self { source })
    }

    pub fn recv_commands(&self) -> Result<CommandIter, io::Error> {
        Ok(CommandIter { source: self.source.try_clone()?, buffer: Vec::new(), data: false })
    }

    pub fn send_command(&mut self, command: ServerCommand) -> Result<(), io::Error> {
        self.source.write_all(&command.into_bytes())?;
        Ok(())
    }

    pub fn shutdown(self) -> Result<(), io::Error> {
        self.source.shutdown(std::net::Shutdown::Both)
    }

    /*pub fn incoming_mails(&mut self) -> MailReceiver<'_, R> {
        MailReceiver { session: self }
    }*/
}

pub struct CommandIter {
    source: TcpStream,
    data: bool,
    buffer: Vec<u8>
}

impl Iterator for CommandIter {
    type Item = Result<ClientCommand, io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut ended = false;
        let mut read_buffer = [0_u8; 2048];
        let mut buffered_data: Vec<u8> = Vec::new();

        while !ended {

            let buffered_line = { // Buffered line with CRLF ending
                let mut cr = false;
                self.buffer.iter().position(|it| if cr {
                        if *it == b'\n' {
                            return true
                        } else {
                            cr = false;
                            return false;
                        }
                    } else if *it == b'\r' {
                        cr = true;
                        return false;
                    } else {
                        return false;
                    })
                    .map(|line_position| 
                        self.buffer.drain(0..line_position+1).collect::<Vec<_>>())
            };

            if let Some(mut buffered_line) = buffered_line {

                if self.data {
                    if buffered_line == b".\r\n" {
                        self.data = false;
                        return Some(Ok(ClientCommand::MailInput(buffered_data)));
                    } else {
                        if buffered_line.starts_with(b".") {
                            buffered_line.remove(0);
                        }
                        buffered_data.append(&mut buffered_line);
                    }
                } else {
                    let command = ClientCommand::from_bytes(&buffered_line).unwrap();

                    if matches!(command, ClientCommand::Data) {
                        self.data = true;
                    }

                    //TODO Handle error
                    return Some(Ok(command));
                }

            } else {
                let result = self.source.read(&mut read_buffer);

                match result {
                    Err(e) => return Some(Err(e)),
                    Ok(byte_red) => {
                        if byte_red > 0 {
                            self.buffer.extend_from_slice(&mut read_buffer[0..byte_red])
                        } else {
                            ended = true
                        }
                    },
                }
            }
        }

        None
    }
}

#[derive(Debug)]
pub enum ClientCommand {
    Hello(String),
    Mail(String),
    Recipient(String),
    Data,
    MailInput(Vec<u8>),
    Quit,
    Reset,
    Verify(String),
    Expand(String),
    Help(Option<String>),
    Noop(Option<String>),
}

impl ClientCommand {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ClientCommandParseError> {

        if bytes[bytes.len()-2..] != *b"\r\n" {
            return Err(ClientCommandParseError::BadEol)
        }

        let options = bytes[0..bytes.len()-2].splitn(2, |it| *it == b' ').collect::<Vec<_>>();

        let Some(command_bytes) = options.get(0) else {
            return Err(ClientCommandParseError::MissingCommand)
        };

        let Ok(mut command_str) = String::from_utf8(command_bytes.to_vec()) else {
            return Err(ClientCommandParseError::InvalidCommandCharacter)
        };

        if !command_str.is_ascii() {
            return Err(ClientCommandParseError::InvalidCommandCharacter)
        }

        command_str = command_str.to_ascii_uppercase();

        match command_str.as_str() {

            "EHLO" => {
                let Some(domain) = options.get(1) else {
                    return Err(ClientCommandParseError::MissingDomain);
                };

                match String::from_utf8(domain.to_vec()) {
                    Ok(domain) => Ok(ClientCommand::Hello(domain)),
                    Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                }
            }

            "MAIL" => {
                let Some(params) = options.get(1) else {
                    return Err(ClientCommandParseError::MissingDomain);
                };

                if ! params.starts_with(b"FROM:") {
                    return Err(ClientCommandParseError::SyntaxInvalid);
                }

                match String::from_utf8(params[5..].to_vec()) {
                    Ok(from) => {
                        if from.contains(" ") {
                            return Err(ClientCommandParseError::SyntaxInvalid);
                        }

                        Ok(ClientCommand::Mail(from))
                    },
                    Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                }
            }

            "RCPT" => {
                let Some(params) = options.get(1) else {
                    return Err(ClientCommandParseError::MissingDomain);
                };

                let params = params.split(|it| *it == b' ').collect::<Vec<_>>();

                let Some(recipient) = params.get(0).and_then(|it| if it.starts_with(b"TO:") { Some(it) } else { None }) else {
                    return Err(ClientCommandParseError::SyntaxInvalid);
                };

                match String::from_utf8(recipient[3..].to_vec()) {
                    Ok(recipient) => Ok(ClientCommand::Recipient(recipient)),
                    Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                }
            }

            "DATA" => {
                Ok(Self::Data)
            }

            "QUIT" => {
                Ok(Self::Quit)
            }

            "RSET" => {
                Ok(Self::Reset)
            }

            "VRFY" => {
                let Some(str) = options.get(1) else {
                    return Err(ClientCommandParseError::MissingParameter);
                };

                match String::from_utf8(str.to_vec()) {
                    Ok(str) => Ok(ClientCommand::Verify(str)),
                    Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                }
            }

            "EXPN" => {
                let Some(str) = options.get(1) else {
                    return Err(ClientCommandParseError::MissingParameter);
                };

                match String::from_utf8(str.to_vec()) {
                    Ok(str) => Ok(ClientCommand::Expand(str)),
                    Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                }
            }

            "HELP" => {

                if let Some(param_str) = options.get(1) {
                    return match String::from_utf8(param_str.to_vec()) {
                        Ok(str) => Ok(ClientCommand::Help(Some(str))),
                        Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                    }
                }

                return Ok(ClientCommand::Help(None))
            }

            "NOOP" => {
                if let Some(param_str) = options.get(1) {
                    return match String::from_utf8(param_str.to_vec()) {
                        Ok(str) => Ok(ClientCommand::Noop(Some(str))),
                        Err(e) => Err(ClientCommandParseError::InvalidCharacter(e))
                    }
                }

                return Ok(ClientCommand::Noop(None))
            }
            
            _ => return Err(ClientCommandParseError::InvalidCommand(command_str))

        }
    }
}

#[derive(Debug, Error)]
pub enum ClientCommandParseError {
    #[error("Command do not end with CRLF line end")]
    BadEol,
    #[error("Missing command")]
    MissingCommand,
    #[error("Missing domain parameter in HELO command")]
    MissingDomain,
    #[error("Syntax invalid")]
    SyntaxInvalid,
    #[error("Invalid ASCII character in command")]
    InvalidCommandCharacter,
    #[error("Invalid character {0}")]
    InvalidCharacter(FromUtf8Error),
    #[error("Invalid command {0}")]
    InvalidCommand(String),
    #[error("Required parameter is missing")]
    MissingParameter
}

#[derive(Debug)]
pub enum ServerCommand {
    OpeningMessage(String),
    HelloOk {
        domain: String,
        greet: Option<String>
    },
    SenderOk,
    RecipientOk,
    StartMailInput,
    MailOk,
    ClosingConnection
}

impl ServerCommand {
    pub fn into_bytes(self) -> Vec<u8> {
        match self {

            ServerCommand::OpeningMessage(domain) => 
                format!("220 {domain} Service ready\r\n").into_bytes(),

            ServerCommand::HelloOk { domain, greet } => {
                    let mut strresult = vec![
                        "250",
                        domain.as_str()
                    ];

                    if let Some(greet) = greet.as_ref() {
                        strresult.push(greet);
                    }

                    strresult.push("\r\n");

                    strresult.join(" ").into_bytes()
                },

            ServerCommand::SenderOk => 
                format!("250 Sender Ok\r\n").into_bytes(),

            ServerCommand::RecipientOk => 
                format!("250 Recipient Ok\r\n").into_bytes(),

            ServerCommand::StartMailInput => 
                format!("354  Start mail input; end with <CRLF>.<CRLF>\r\n").into_bytes(),

            ServerCommand::MailOk => 
                format!("250 Mail Ok\r\n").into_bytes(),

            ServerCommand::ClosingConnection => 
                format!("221 Closing connection\r\n").into_bytes(),
        }
    }
}

/*pub struct MailReceiver<'a, R: Read + Write> {
    session: &'a mut SmtpSession<R>
}*/

// #[derive(Debug)]
// pub struct Mail {

// }