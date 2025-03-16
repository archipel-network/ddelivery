use std::{io::{self, Read, Write}, net::TcpStream, string::FromUtf8Error};

use log::error;
use thiserror::Error;

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

    fn recv_commands(&self) -> Result<CommandIter, io::Error> {
        Ok(CommandIter { source: self.source.try_clone()?, buffer: Vec::new(), data: false })
    }

    fn send_command(&mut self, command: ServerCommand) -> Result<(), io::Error> {
        self.source.write_all(&command.into_bytes())?;
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), io::Error> {
        self.source.shutdown(std::net::Shutdown::Both)
    }

    pub fn into_mail_iter(self) -> Result<MailReceiver, io::Error> {
        MailReceiver::new(self)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Err(e) = self.shutdown() {
            error!("Failed to shutdown session {e}")
        }
    }
}

pub struct CommandIter {
    source: TcpStream,
    data: bool,
    buffer: Vec<u8>
}

impl Iterator for CommandIter {
    type Item = Result<ClientCommand, SmtpError>;

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
                    let command = match ClientCommand::from_bytes(&buffered_line) {
                        Ok(it) => it,
                        Err(e) => {
                            return Some(Err(SmtpError::Command(e)));
                        }
                    };

                    if matches!(command, ClientCommand::Data) {
                        self.data = true;
                    }

                    return Some(Ok(command));
                }

            } else {
                let result = self.source.read(&mut read_buffer);

                match result {
                    Err(e) => return Some(Err(SmtpError::Io(e))),
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

#[derive(Debug, Error)]
pub enum SmtpError {
    #[error("IO error : {0}")]
    Io(#[from] io::Error),
    #[error("Command parsing error : {0}")]
    Command(#[from] ClientCommandParseError),
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
    NoopOk,
    ResetOk,
    StartMailInput,
    MailOk,
    ClosingConnection,
    SyntaxError,
    CommandUnrecognized,
    LocalError(String),
    CommandNotImplemented,
    ParameterNotImplemented,
    BadSequenceOfCommand(String)
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

            ServerCommand::SyntaxError => 
                format!("501 Syntax error\r\n").into_bytes(),

            ServerCommand::CommandNotImplemented => 
                format!("502 Not implemented\r\n").into_bytes(),

            ServerCommand::ParameterNotImplemented => 
                format!("504 Not implemented\r\n").into_bytes(),

            ServerCommand::LocalError(text) => 
                format!("451 {text}\r\n").into_bytes(),

            ServerCommand::CommandUnrecognized => 
                format!("500 Command unrecognized\r\n").into_bytes(),

            ServerCommand::BadSequenceOfCommand(text) => 
                format!("503 Bad sequence of command. {text}\r\n").into_bytes(),

            ServerCommand::NoopOk => 
                format!("250 OK\r\n").into_bytes(),

            ServerCommand::ResetOk => 
                format!("250 OK\r\n").into_bytes(),
        }
    }
}

pub struct MailReceiver {
    session: Session,
    commands: CommandIter
}

impl MailReceiver {
    pub fn new(smtp_session: Session) -> Result<Self, io::Error> {
        let command_iter = match smtp_session.recv_commands() {
            Ok(iter) => iter,
            Err(e) => return Err(e)
        };

        Ok(Self { session: smtp_session, commands: command_iter })
    }
}

impl Iterator for MailReceiver {
    type Item = Result<Mail, io::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut current_mail: Option<Mail> = None;

        for command in &mut self.commands {
            match command {
                Ok(command) => {
                    match command {

                        ClientCommand::Hello(domain) => if let Err(e) = self.session.send_command(ServerCommand::HelloOk { 
                                domain,
                                greet: Some("delayed greetings !".to_owned())
                            }) {
                                return Some(Err(e))
                        },

                        ClientCommand::Mail(from_address) => {
                            match &mut current_mail {
                                Some(_) => {
                                    if let Err(e) = self.session.send_command(ServerCommand::BadSequenceOfCommand("Mail sequence already started".to_owned())) {
                                        return Some(Err(e))
                                    }
                                },
                                None => {
                                    current_mail = Some(Mail::new(from_address));
                                    if let Err(e) = self.session.send_command(ServerCommand::SenderOk) {
                                        return Some(Err(e));
                                    }
                                }
                            }
                        },

                        ClientCommand::Recipient(recipient_address) => {
                            match &mut current_mail {
                                Some(m) => {
                                    m.receipients.push(recipient_address);
                                    if let Err(e) = self.session.send_command(ServerCommand::RecipientOk) {
                                        return Some(Err(e))
                                    }
                                },
                                None => {
                                    if let Err(e) = self.session.send_command(ServerCommand::BadSequenceOfCommand("No mail sequence. Begin with a MAIL command".to_owned())) {
                                        return Some(Err(e))
                                    }
                                }
                            }
                        },

                        ClientCommand::Data => {
                            if let Err(e) = self.session.send_command(ServerCommand::StartMailInput) {
                                return Some(Err(e))
                            }
                        },

                        ClientCommand::MailInput(content) => {
                            match current_mail.take() {
                                Some(mut m) => {
                                    m.content = content;
                                    if let Err(e) = self.session.send_command(ServerCommand::MailOk) {
                                        return Some(Err(e));
                                    }
                                    return Some(Ok(m));
                                },
                                None => {
                                    if let Err(e) = self.session.send_command(ServerCommand::BadSequenceOfCommand("No mail sequence. Begin with a MAIL command".to_owned())) {
                                        return Some(Err(e))
                                    }
                                }
                            }
                        },

                        ClientCommand::Quit => {
                            if let Err(e) = self.session.send_command(ServerCommand::ClosingConnection) {
                                return Some(Err(e))
                            }
                            break;
                        },

                        ClientCommand::Expand(_) => {
                            if let Err(e) = self.session.send_command(ServerCommand::CommandNotImplemented) {
                                return Some(Err(e))
                            }
                        },

                        ClientCommand::Verify(_) => {
                            if let Err(e) = self.session.send_command(ServerCommand::CommandNotImplemented) {
                                return Some(Err(e))
                            }
                        },

                        ClientCommand::Noop(_) => {
                            if let Err(e) = self.session.send_command(ServerCommand::NoopOk) {
                                return Some(Err(e))
                            }
                        },

                        ClientCommand::Reset => {
                            current_mail = None;
                            if let Err(e) = self.session.send_command(ServerCommand::ResetOk) {
                                return Some(Err(e))
                            }
                        },

                        ClientCommand::Help(_) => {
                            if let Err(e) = self.session.send_command(ServerCommand::CommandNotImplemented) {
                                return Some(Err(e))
                            }
                        }
                    }
                },
                Err(SmtpError::Command(e)) => {
                    match e {
                        ClientCommandParseError::BadEol |
                        ClientCommandParseError::InvalidCharacter(_) |
                        ClientCommandParseError::InvalidCommandCharacter |
                        ClientCommandParseError::SyntaxInvalid |
                        ClientCommandParseError::MissingDomain |
                        ClientCommandParseError::MissingParameter => {
                            if let Err(e) = self.session.send_command(ServerCommand::SyntaxError) {
                                return Some(Err(e))
                            }
                        },
                        ClientCommandParseError::MissingCommand |
                        ClientCommandParseError::InvalidCommand(_) => {
                            if let Err(e) = self.session.send_command(ServerCommand::CommandUnrecognized) {
                                return Some(Err(e))
                            }
                        }
                    }
                }
                Err(SmtpError::Io(e)) => error!("Failed to read commands : {e}")
            }
            
        }
        if let Err(e) = self.session.shutdown() {
            return Some(Err(e))
        }
        None
    }
}

#[derive(Debug)]
pub struct Mail {
    from: String,
    receipients: Vec<String>,
    content: Vec<u8>
}

impl Mail {
    pub fn new(from_address: String) -> Self {
        Self { from: from_address, receipients: Vec::new(), content: Vec::new() }
    }
}