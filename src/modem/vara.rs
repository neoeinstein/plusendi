use std::collections::VecDeque;
use std::fmt;
use std::fmt::{Formatter, Write};
use nom::{AsBytes, Finish, IResult, ParseTo};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, Interest};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio::sync::mpsc::{error::TryRecvError, Sender, Receiver};


//use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use crate::{StationId, StationIdRef};

pub struct Vara {
    my_call: StationId,
    control: TcpStream,
    data: TcpStream,
    registered: Option<Registration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    Unregistered,
    Registered,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Listen(ListenMode),
    // CallCQ(CQFrame),
    Connect(ConnectCommand),
    Disconnect,
    Abort,
    SetCall(MyCallSigns),
    SetCompression(CompressionMode),
    SetBandwidth(BandwidthMode),
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listen(mode) => write!(f, "LISTEN {}", mode)?,
            // Self::CallCQ(frame) => write!(f, "CQFRAME {}", frame)?,
            Self::Connect(connect) => write!(f, "CONNECT {}", connect)?,
            Self::Disconnect => f.write_str("DISCONNECT")?,
            Self::Abort => f.write_str("ABORT")?,
            Self::SetCall(calls) => write!(f, "MYCALL {}", calls)?,
            Self::SetCompression(mode) => write!(f, "COMPRESSION {}", mode)?,
            Self::SetBandwidth(mode) => write!(f, "BW{}", mode)?,
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionMode {
    Off,
    Text,
    Files,
}

impl fmt::Display for CompressionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Off => "OFF",
            Self::Text => "TEXT",
            Self::Files => "FILES",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandwidthMode {
    Narrow,
    Wide,
    Tactical,
}

impl fmt::Display for BandwidthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Narrow => "500",
            Self::Wide => "2300",
            Self::Tactical => "2750",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MyCallSigns(pub StationId, pub Vec<StationId>);

impl fmt::Display for MyCallSigns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)?;
        for id in &self.1 {
            write!(f, " {}", id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenMode {
    Disable,
    CQ,
    Enable,
}

impl fmt::Display for ListenMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Disable => "OFF",
            Self::CQ => "CQ",
            Self::Enable => "ON",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update<'a> {
    Heartbeat,
    Buffer { bytes_remaining: u16 },
    Busy(BusyState),
    Connection(ConnectionState<'a>),
    TransceiverControl(TransceiverCommand),
    Registered { my_call: &'a StationIdRef },
    RemoteRegistration(Registration),
    CommandResult(CommandResult),
    // CQFrame(CQFrame<'a>),
}

fn update(data: &[u8]) -> IResult<&[u8], Update> {
    nom::branch::alt((
        nom::combinator::value(Update::Heartbeat, nom::bytes::complete::tag("IAMALIVE")),
        buffer,
        nom::combinator::map(busy_state, Update::Busy),
        nom::combinator::map(connection_state, Update::Connection),
        nom::combinator::map(transmit_state, Update::TransceiverControl),
        registered,
        // nom::combinator::map(remote_registration, Update::RemoteRegistration),
        nom::combinator::map(command_result, Update::CommandResult),
    ))(data)
}

fn buffer(data: &[u8]) -> IResult<&[u8], Update> {
    let (remaining, bytes_remaining) = nom::sequence::preceded(
        nom::bytes::complete::tag("BUFFER "),
        nom::combinator::map_res(nom::bytes::complete::take_while(nom::character::is_digit), |x: &[u8]| u16::from_str_radix(unsafe { std::str::from_utf8_unchecked(x) }, 10))
    )(data)?;
    Ok((remaining, Update::Buffer { bytes_remaining }))
}

fn registered(data: &[u8]) -> IResult<&[u8], Update> {
    nom::sequence::preceded(
        nom::bytes::complete::tag("REGISTERED "),
        nom::combinator::map(crate::types::callsign, |my_call: &StationIdRef| Update::Registered { my_call })
    )(data)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectCommand {
    pub origin: StationId,
    pub target: StationId,
    pub path: ConnectPath,
}

impl fmt::Display for ConnectCommand {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}{}", self.origin, self.target, self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectPath {
    Direct,
    OneHop {
        digipeater: StationId,
    },
    TwoHops {
        first: StationId,
        second: StationId,
    },
}

impl fmt::Display for ConnectPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ConnectPath::Direct => Ok(()),
            ConnectPath::OneHop { digipeater } => write!(f, " via {}", digipeater),
            ConnectPath::TwoHops { first, second } => write!(f, "via {} {}", first, second),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CQFrame<'a> {
    cq_station: &'a StationIdRef,
    via: VaraCQPath<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaraCQPath<'a> {
    Satellite,
    HF(BandwidthMode),
    FM(VaraFMPath<'a>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaraFMPath<'a> {
    Direct,
    OneHop {
        digipeater: &'a StationIdRef,
    },
    TwoHops {
        first_digipeater: &'a StationIdRef,
        second_digipeater: &'a StationIdRef,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandResult {
    Ok,
    Wrong,
}

fn command_result(data: &[u8]) -> IResult<&[u8], CommandResult> {
    nom::branch::alt((nom::combinator::value(CommandResult::Ok, nom::bytes::complete::tag("OK")), nom::combinator::value(CommandResult::Wrong, nom::bytes::complete::tag("WRONG"))))(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransceiverCommand {
    Receive,
    Transmit,
}

fn transmit_state(data: &[u8]) -> IResult<&[u8], TransceiverCommand> {
    let (rest, is_transmit) = nom::sequence::preceded(nom::bytes::complete::tag("PTT "), on_or_off)(data)?;
    Ok((rest, if is_transmit { TransceiverCommand::Transmit } else { TransceiverCommand::Receive }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyState {
    NotBusy,
    Busy,
}

fn busy_state(data: &[u8]) -> IResult<&[u8], BusyState> {
    let (rest, is_busy) = nom::sequence::preceded(nom::bytes::complete::tag("BUSY "), on_or_off)(data)?;
    Ok((rest, if is_busy { BusyState::Busy } else { BusyState::NotBusy }))
}

fn on_or_off(data: &[u8]) -> IResult<&[u8], bool> {
    nom::branch::alt((nom::combinator::value(true, nom::bytes::complete::tag("ON")), nom::combinator::value(false, nom::bytes::complete::tag("OFF"))))(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState<'a> {
    Disconnected,
    Pending,
    Canceled,
    Connected { other_station: &'a StationIdRef },
}

fn disconnected(data: &[u8]) -> IResult<&[u8], ConnectionState> {
    nom::combinator::value(ConnectionState::Disconnected, nom::bytes::complete::tag("DISCONNECTED"))(data)
}

fn pending(data: &[u8]) -> IResult<&[u8], ConnectionState> {
    nom::combinator::value(ConnectionState::Pending, nom::bytes::complete::tag("PENDING"))(data)
}

fn canceled(data: &[u8]) -> IResult<&[u8], ConnectionState> {
    nom::combinator::value(ConnectionState::Canceled, nom::bytes::complete::tag("CANCELPENDING"))(data)
}

fn connected(data: &[u8]) -> IResult<&[u8], ConnectionState> {
    let (rest, _) = nom::bytes::complete::tag("CONNECTED")(data)?;
    let (rest, other_station) = crate::types::callsign(rest)?;
    Ok((rest, ConnectionState::Connected { other_station }))
}

fn connection_state(data: &[u8]) -> IResult<&[u8], ConnectionState> {
    nom::branch::alt((disconnected, pending, canceled, connected))(data)
}
//
// impl Vara {
//     pub fn new<S: ToSocketAddrs>(my_call: StationId, control: S) -> Result<Self, std::io::Error> {
//         let control = control.to_socket_addrs()?.next().ok_or(std::io::Error::new(std::io::ErrorKind::InvalidInput, "no address found"))?;
//         let mut data = control;
//         data.set_port(control.port() + 1);
//
//         let control = TcpStream::connect(control)?;
//         let data = TcpStream::connect(data)?;
//
//         let mut slf = Self {
//             my_call,
//             control,
//             data,
//             registered: None,
//         };
//
//         slf.send_command(&format!("MYCALL {}", my_call))?;
//
//         Ok(slf)
//     }
//
//     pub(crate) fn send_command(&mut self, data: &str) -> Result<(), std::io::Error> {
//         self.control.write_all(data.as_bytes())?;
//         self.control.write(b"\r")?;
//         self.control.flush()?;
//
//         let mut buf = Vec::with_capacity(32);
//         let mut byte_iter = std::io::Read::by_ref(&mut self.control).bytes();
//         while let Some(b) = byte_iter.next() {
//             let b = b?;
//             if b == b'\r' {
//                 break;
//             } else {
//                 buf.push(b);
//             }
//         }
//         drop(byte_iter);
//
//         if buf == b"OK" {
//             Ok(())
//         } else if buf == b"WRONG" {
//             Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "wrong"))
//         } else {
//             Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("unexpected response: {}", std::str::from_utf8(&buf).unwrap())))
//         }
//     }
// }
//
// pub struct VaraConnection<'a> {
//     parent: &'a mut Vara,
// }
//
// impl<'a> Drop for VaraConnection<'a> {
//     fn drop(&mut self) {
//         match self.send_command("DISCONNECT") {
//             Ok(()) => (),
//             Err(err) => eprintln!("Error during drop! {}", err),
//         }
//     }
// }
//
// impl<'a> super::Modem<'a> for Vara {
//     type Connection = VaraConnection<'a>;
//     type ConnectionError = std::io::Error;
//
//     fn connect(&'a mut self, station: &StationIdRef) -> Result<Self::Connection, Self::ConnectionError> {
//         self.send_command(&format!("CONNECT KC1GSL {}", station))?;
//         Ok(VaraConnection {
//             parent: self,
//         })
//     }
// }

fn line(data: &[u8]) -> IResult<&[u8], &[u8]> {
    nom::sequence::terminated(nom::bytes::streaming::take_until1("\r"), nom::bytes::streaming::tag("\r"))(data)
}

pub async fn manage_modem_thread(mut rx: Receiver<(Command, tokio::sync::oneshot::Sender<Result<(), ()>>)>, /*tx: Sender<Update<'static>>, */mut stream: TcpStream) -> color_eyre::Result<()> {
    let mut cmd_buffer = String::with_capacity(32);
    let mut upd_buffer = bytes::BytesMut::with_capacity(32);
    let mut response_queue = VecDeque::with_capacity(4);
    let mut command_active = true;

    while command_active {
        tokio::select!(
            recv = rx.recv() => {
                if let Some((command, reply)) = recv {
                    response_queue.push_back(reply);
                    stream.writable().await?;
                    cmd_buffer.clear();
                    write!(&mut cmd_buffer, "{}\r", command).unwrap();
                    tracing::trace!(command = cmd_buffer.as_str(), "sending command");
                    stream.write_all(cmd_buffer.as_bytes()).await?;
                } else {
                    command_active = false
                }
            },
            ready = stream.readable() => {
                let results = do_a_thing(&mut stream, &mut upd_buffer)?;
                for result in results {
                    if let Some(reply) = response_queue.pop_front() {
                        let _ = reply.send(result);
                    } else {
                        tracing::warn!("mismatched reply queue");
                    }
                }
            }
        );
    }
    tracing::info!(expected_replies = response_queue.len(), "command input closed");
    while !response_queue.is_empty() {
        let results = do_a_thing(&mut stream, &mut upd_buffer)?;
        for result in results {
            if let Some(reply) = response_queue.pop_front() {
                let _ = reply.send(result);
            } else {
                tracing::warn!("mismatched reply queue");
            }
        }
    }
    tracing::info!("all replies sent; exiting command loop");
    Ok(())
}

#[tracing::instrument(skip(stream, upd_buffer), err)]
fn do_a_thing(stream: &mut TcpStream, upd_buffer: &mut bytes::BytesMut) -> color_eyre::Result<Vec<Result<(), ()>>> {
    let mut to_acknowledge = Vec::new();
    match stream.try_read_buf(upd_buffer) {
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(to_acknowledge),
        Err(err) => return Err(err.into()),
        Ok(bytes) => {
            tracing::trace!(bytes, "received bytes from command port");
        }
    }

    let retain_after = {
        let mut data = upd_buffer.as_bytes();
        while data.len() > 0 {
            match line(&data) {
                Ok((remaining, line)) => {
                    tracing::trace!(line = std::str::from_utf8(line).unwrap(), remaining = std::str::from_utf8(remaining).unwrap(), "received complete line");
                    data = remaining;

                    match nom::combinator::all_consuming(update)(line).map_err(|e| e.map_input(|i| String::from_utf8(i.into()).unwrap())).finish() {
                        Ok((_ , update)) => {
                            tracing::debug!(?update, "received update");
                            match update {
                                Update::CommandResult(result) => {
                                    if result == CommandResult::Ok {
                                        to_acknowledge.push(Ok(()))
                                    } else {
                                        to_acknowledge.push(Err(()))
                                    }
                                }
                                Update::Heartbeat => {
                                    tracing::debug!("received heartbeat");
                                }
                                Update::TransceiverControl(control) => {
                                    //tracing::debug!("")
                                }
                                _ => {}
                            }
                        }
                        Err(err) => {
                            return Err(err.into());
                        }
                    }
                },
                Err(err) if err.is_incomplete() => {
                    tracing::trace!(buffer = std::str::from_utf8(data).unwrap(), "incomplete");
                    break
                },
                Err(err) => {
                    return Err(err.to_owned().into())
                },
            }
        }
        upd_buffer.len() - data.len()
    };
    if retain_after == upd_buffer.len() {
      upd_buffer.clear();
    } else if retain_after > 0 {
        let new = upd_buffer.split_off(retain_after);
        std::mem::replace(upd_buffer, new);
        tracing::trace!(bytes = upd_buffer.len(), "retained incomplete parts");
    }
    Ok(to_acknowledge)
}
