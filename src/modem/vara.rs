use std::collections::VecDeque;
use std::fmt;
use std::fmt::{Debug, Formatter, Write};
use std::future::Future;
use std::io::{Error, IoSlice};
use std::num::NonZeroU16;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use nom::{AsBytes, Finish, IResult};
use nom::error::VerboseError;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{Sender, Receiver};
use crate::parser::MappableParserInputError;


//use std::io::{self, BufRead, BufReader, BufWriter, Read, Write};
use crate::{StationId, StationIdRef};


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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TncResponse<'a> {
    Update(Update<'a>),
    CommandResult(CommandResult),
}

fn tnc_response(data: &[u8]) -> IResult<&[u8], TncResponse, VerboseError<&[u8]>> {
    nom::branch::alt((
        nom::combinator::map(command_result, TncResponse::CommandResult),
        nom::combinator::map(update, TncResponse::Update),
    ))(data)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update<'a> {
    Heartbeat,
    Buffer { bytes_remaining: usize },
    Busy(BusyState),
    Connection(ConnectionState<'a>),
    TransceiverControl(TransceiverCommand),
    Registered { my_call: &'a StationIdRef },
    RemoteRegistration(Registration),
    // CQFrame(CQFrame<'a>),
}

fn update(data: &[u8]) -> IResult<&[u8], Update, VerboseError<&[u8]>> {
    nom::branch::alt((
        nom::combinator::value(Update::Heartbeat, nom::bytes::complete::tag("IAMALIVE")),
        buffer,
        nom::combinator::map(busy_state, Update::Busy),
        nom::combinator::map(connection_state, Update::Connection),
        nom::combinator::map(transmit_state, Update::TransceiverControl),
        registered,
        // nom::combinator::map(remote_registration, Update::RemoteRegistration),
    ))(data)
}

fn buffer(data: &[u8]) -> IResult<&[u8], Update, VerboseError<&[u8]>> {
    let (remaining, bytes_remaining) = nom::sequence::preceded(
        nom::bytes::complete::tag("BUFFER "),
        nom::combinator::map_res(nom::bytes::complete::take_while(nom::character::is_digit), |x: &[u8]| usize::from_str_radix(unsafe { std::str::from_utf8_unchecked(x) }, 10))
    )(data)?;
    Ok((remaining, Update::Buffer { bytes_remaining }))
}

fn registered(data: &[u8]) -> IResult<&[u8], Update, VerboseError<&[u8]>> {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CQFrame<'a> {
    cq_station: &'a StationIdRef,
    via: VaraCQPath<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VaraCQPath<'a> {
    Satellite,
    HF(BandwidthMode),
    FM(VaraFMPath<'a>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

fn command_result(data: &[u8]) -> IResult<&[u8], CommandResult, VerboseError<&[u8]>> {
    nom::branch::alt((nom::combinator::value(CommandResult::Ok, nom::bytes::complete::tag("OK")), nom::combinator::value(CommandResult::Wrong, nom::bytes::complete::tag("WRONG"))))(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransceiverCommand {
    Receive,
    Transmit,
}

fn transmit_state(data: &[u8]) -> IResult<&[u8], TransceiverCommand, VerboseError<&[u8]>> {
    let (rest, is_transmit) = nom::sequence::preceded(nom::bytes::complete::tag("PTT "), on_or_off)(data)?;
    Ok((rest, if is_transmit { TransceiverCommand::Transmit } else { TransceiverCommand::Receive }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyState {
    NotBusy,
    Busy,
}

fn busy_state(data: &[u8]) -> IResult<&[u8], BusyState, VerboseError<&[u8]>> {
    let (rest, is_busy) = nom::sequence::preceded(nom::bytes::complete::tag("BUSY "), on_or_off)(data)?;
    Ok((rest, if is_busy { BusyState::Busy } else { BusyState::NotBusy }))
}

fn on_or_off(data: &[u8]) -> IResult<&[u8], bool, VerboseError<&[u8]>> {
    nom::branch::alt((nom::combinator::value(true, nom::bytes::complete::tag("ON")), nom::combinator::value(false, nom::bytes::complete::tag("OFF"))))(data)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState<'a> {
    Disconnected,
    Pending,
    Canceled,
    Connected {
        my_station: &'a StationIdRef,
        other_station: &'a StationIdRef,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionStateOwned {
    Disconnected,
    Pending,
    Canceled,
    Connected {
        my_station: StationId,
        other_station: StationId,
    },
}

impl ConnectionStateOwned {
    fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    fn is_disconnected(&self) -> bool {
        matches!(self, Self::Disconnected)
    }
}

impl<'a> ConnectionState<'a> {
    fn into_owned(self) -> ConnectionStateOwned {
        match self {
            Self::Disconnected => ConnectionStateOwned::Disconnected,
            Self::Pending => ConnectionStateOwned::Pending,
            Self::Canceled => ConnectionStateOwned::Canceled,
            Self::Connected { my_station, other_station } => {
                ConnectionStateOwned::Connected {
                    my_station: my_station.to_owned(),
                    other_station: other_station.to_owned(),
                }
            }
        }
    }
}

fn disconnected(data: &[u8]) -> IResult<&[u8], ConnectionState, VerboseError<&[u8]>> {
    nom::combinator::value(ConnectionState::Disconnected, nom::bytes::complete::tag("DISCONNECTED"))(data)
}

fn pending(data: &[u8]) -> IResult<&[u8], ConnectionState, VerboseError<&[u8]>> {
    nom::combinator::value(ConnectionState::Pending, nom::bytes::complete::tag("PENDING"))(data)
}

fn canceled(data: &[u8]) -> IResult<&[u8], ConnectionState, VerboseError<&[u8]>> {
    nom::combinator::value(ConnectionState::Canceled, nom::bytes::complete::tag("CANCELPENDING"))(data)
}

fn connected(data: &[u8]) -> IResult<&[u8], ConnectionState, VerboseError<&[u8]>> {
    let (rest, (my_station, other_station)) = nom::sequence::preceded(
        nom::bytes::complete::tag("CONNECTED "),
        nom::sequence::separated_pair(
            crate::types::callsign,
            nom::bytes::complete::tag(" "),
            crate::types::callsign,
        ),
    )(data)?;
    Ok((rest, ConnectionState::Connected { my_station, other_station }))
}

fn connection_state(data: &[u8]) -> IResult<&[u8], ConnectionState, VerboseError<&[u8]>> {
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

fn line(data: &[u8]) -> IResult<&[u8], &[u8], VerboseError<&[u8]>> {
    nom::sequence::terminated(nom::bytes::streaming::take_until1("\r"), nom::bytes::streaming::tag("\r"))(data)
}

#[tracing::instrument(skip(rx, tx, stream), err)]
async fn manage_modem_thread(mut rx: Receiver<(Command, tokio::sync::oneshot::Sender<CommandResult>)>, mut tx: TncStatusSender, mut stream: TcpStream) -> color_eyre::Result<()> {
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
            _ = stream.readable() => {
                let results = do_a_thing(&mut stream, &mut upd_buffer, &mut tx)?;
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
        let results = do_a_thing(&mut stream, &mut upd_buffer, &mut tx)?;
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

fn stringify_input<T: std::fmt::Display>(error: nom::Err<VerboseError<T>>) -> nom::Err<VerboseError<String>> {
    error.map(|err| {
        VerboseError {
            errors: err.errors.into_iter().map(|e| (e.0.to_string(), e.1)).collect()
        }
    })
}

#[tracing::instrument(skip(stream, upd_buffer, tx), err)]
fn do_a_thing(stream: &mut TcpStream, upd_buffer: &mut bytes::BytesMut, tx: &mut TncStatusSender) -> color_eyre::Result<Vec<CommandResult>> {
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
            match line(&data).try_map_into_str().map_err(stringify_input) {
                Ok((remaining, line)) => {
                    tracing::trace!(line = std::str::from_utf8(line).unwrap(), remaining = std::str::from_utf8(remaining).unwrap(), "received complete line");
                    data = remaining;

                    match nom::combinator::all_consuming(tnc_response)(line).try_map_into_str().map_err(stringify_input).finish() {
                        Ok((_ , response)) => {
                            tracing::debug!(?response, "received tnc data");
                            match response {
                                TncResponse::CommandResult(result) => {
                                    to_acknowledge.push(result);
                                }
                                TncResponse::Update(update) => {
                                    match update {
                                        Update::Heartbeat => {
                                            tx.last_heartbeat.send_replace(std::time::Instant::now());
                                        }
                                        Update::Buffer { bytes_remaining } => {
                                            tx.buffer.send_replace(bytes_remaining);
                                        }
                                        Update::Busy(state) => {
                                            tx.busy_state.send_replace(state);
                                        }
                                        Update::Registered { my_call } => {
                                            tx.calls.insert(my_call.to_owned());
                                            tx.registered_calls.send_replace(tx.calls.clone());
                                        }
                                        Update::Connection(state) => {
                                            tx.connection.send_replace(state.into_owned());
                                        }
                                        Update::RemoteRegistration(registration) => {
                                            tx.remote_registration.send_replace(registration);
                                        }
                                        Update::TransceiverControl(control) => {
                                            tx.transceiver_control.send_replace(control);
                                        }
                                    }
                                }
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
                    return Err(err.into())
                },
            }
        }
        upd_buffer.len() - data.len()
    };
    if retain_after == upd_buffer.len() {
      upd_buffer.clear();
    } else if retain_after > 0 {
        let new = upd_buffer.split_off(retain_after);
        *upd_buffer = new;
        tracing::trace!(bytes = upd_buffer.len(), "retained incomplete parts");
    }
    Ok(to_acknowledge)
}

#[derive(Debug)]
#[pin_project::pin_project]
pub struct VaraTnc {
    data: TcpStream,
    control_channel: Sender<(Command, tokio::sync::oneshot::Sender<CommandResult>)>,
    status: TncStatusReceiver,
    managing_thread: tokio::task::JoinHandle<color_eyre::Result<()>>,
}

fn channel() -> (TncStatusSender, TncStatusReceiver) {
    use tokio::sync::watch::channel;
    let (busy_tx, busy_rx) = channel(BusyState::NotBusy);
    let (buffer_tx, buffer_rx) = channel(0);
    let (registered_calls_tx, registered_calls_rx) = channel(std::collections::HashSet::new());
    let (heartbeat_tx, heartbeat_rx) = channel(Instant::now());
    let (connection_tx, connection_rx) = channel(ConnectionStateOwned::Disconnected);
    let (transceiver_tx, transceiver_rx) = channel(TransceiverCommand::Receive);
    let (remote_registration_tx, remote_registration_rx) = channel(Registration::Unregistered);

    let sender = TncStatusSender {
        calls: Default::default(),
        busy_state: busy_tx,
        buffer: buffer_tx,
        registered_calls: registered_calls_tx,
        last_heartbeat: heartbeat_tx,
        connection: connection_tx,
        transceiver_control: transceiver_tx,
        remote_registration: remote_registration_tx,
    };

    let receiver = TncStatusReceiver {
        busy_state: busy_rx,
        buffer: buffer_rx,
        registered_calls: registered_calls_rx,
        last_heartbeat: heartbeat_rx,
        connection: connection_rx,
        transceiver_control: transceiver_rx,
        remote_registration: remote_registration_rx,
    };

    (sender, receiver)
}

#[derive(Debug)]
struct TncStatusSender {
    calls: std::collections::HashSet<StationId>,
    busy_state: tokio::sync::watch::Sender<BusyState>,
    buffer: tokio::sync::watch::Sender<usize>,
    registered_calls: tokio::sync::watch::Sender<std::collections::HashSet<StationId>>,
    last_heartbeat: tokio::sync::watch::Sender<std::time::Instant>,
    connection: tokio::sync::watch::Sender<ConnectionStateOwned>,
    transceiver_control: tokio::sync::watch::Sender<TransceiverCommand>,
    remote_registration: tokio::sync::watch::Sender<Registration>,
}

#[derive(Debug)]
struct TncStatusReceiver {
    busy_state: tokio::sync::watch::Receiver<BusyState>,
    buffer: tokio::sync::watch::Receiver<usize>,
    registered_calls: tokio::sync::watch::Receiver<std::collections::HashSet<StationId>>,
    last_heartbeat: tokio::sync::watch::Receiver<std::time::Instant>,
    connection: tokio::sync::watch::Receiver<ConnectionStateOwned>,
    transceiver_control: tokio::sync::watch::Receiver<TransceiverCommand>,
    remote_registration: tokio::sync::watch::Receiver<Registration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaraTncBuilder {
    host: std::net::IpAddr,
    control_port: NonZeroU16,
    data_port: Option<NonZeroU16>,
}

impl VaraTncBuilder {
    pub async fn build(&mut self) -> std::io::Result<VaraTnc> {
        if self.control_port.get() == u16::MAX && self.data_port.is_none() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid control port with unspecified data port"));
        }

        let control = TcpStream::connect((self.host, self.control_port.get())).await?;
        let data = TcpStream::connect((self.host, self.data_port.unwrap_or_else(|| NonZeroU16::new(self.control_port.get() + 1).unwrap()).get())).await?;

        let (control_tx, control_rx) = tokio::sync::mpsc::channel(1);
        let (status_tx, status_rx) = channel();

        let managing_thread = tokio::spawn(manage_modem_thread(control_rx, status_tx, control));

        Ok(VaraTnc {
            data,
            control_channel: control_tx,
            status: status_rx,
            managing_thread,
        })
    }

    pub fn host(&mut self, host: std::net::IpAddr) -> &mut Self {
        self.host = host;
        self
    }

    pub fn control_port(&mut self, port: NonZeroU16) -> &mut Self {
        self.control_port = port;
        self
    }

    pub fn data_port(&mut self, port: NonZeroU16) -> &mut Self {
        self.data_port = Some(port);
        self
    }
}

impl From<StationId> for MyCallSigns {
    fn from(s: StationId) -> Self {
        Self(s, Vec::new())
    }
}

impl VaraTnc {
    pub fn builder() -> VaraTncBuilder {
        VaraTncBuilder {
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            control_port: 8300.try_into().unwrap(),
            data_port: None,
        }
    }

    async fn send_command(&self, command: Command) -> color_eyre::Result<()> {
        let (cmd_tx, cmd_rx) = tokio::sync::oneshot::channel();

        self.control_channel.send((command, cmd_tx)).await?;

        if cmd_rx.await? == CommandResult::Wrong {
            return Err(color_eyre::eyre::eyre!("failed to send connect command to tnc"));
        }

        Ok(())
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn send_callsign<T: Into<MyCallSigns> + Debug>(&self, cs: T) -> color_eyre::Result<()> {
        self.send_command(Command::SetCall(cs.into())).await
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn send_compression(&self, mode: CompressionMode) -> color_eyre::Result<()> {
        self.send_command(Command::SetCompression(mode)).await
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn send_bandwidth(&self, mode: BandwidthMode) -> color_eyre::Result<()> {
        self.send_command(Command::SetBandwidth(mode)).await
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn send_disconnect(&self) -> color_eyre::Result<()> {
        self.send_command(Command::Disconnect).await
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn send_abort(&self) -> color_eyre::Result<()> {
        self.send_command(Command::Abort).await
    }

    pub fn subscribe_rig_command(&self) -> tokio::sync::watch::Receiver<TransceiverCommand> {
        self.status.transceiver_control.clone()
    }

    pub fn remote_registration(&self) -> Registration {
        *self.status.remote_registration.borrow()
    }

    pub fn last_heartbeat(&self) -> Instant {
        *self.status.last_heartbeat.borrow()
    }

    pub fn local_registration(&self, station: &StationIdRef) -> Registration {
        if self.status.registered_calls.borrow().contains(station) {
            Registration::Registered
        } else {
            Registration::Unregistered
        }
    }

    pub fn buffer(&self) -> usize {
        *self.status.buffer.borrow()
    }

    pub fn busy_state(&self) -> BusyState {
        *self.status.busy_state.borrow()
    }

    #[tracing::instrument(skip(self), err)]
    pub async fn connect<'a>(&'a mut self, from: StationId, to: StationId) -> color_eyre::Result<VaraStream<'a>> {
        self.send_command(Command::Connect(ConnectCommand {
            origin: from,
            target: to,
            path: ConnectPath::Direct,
        })).await?;

        let (force_dc, force_disconnect) = tokio::sync::oneshot::channel();
        let cloned_control = self.control_channel.clone();
        let _force_dc = tokio::spawn(async move {
            if let Ok(()) = force_disconnect.await {
                let (tx, rx) = tokio::sync::oneshot::channel();
                let _ = cloned_control.send((Command::Disconnect, tx)).await;
                let _ = rx.await;
            }
        });

        self.status.connection.changed().await?;

        if self.status.connection.borrow().is_connected() {
            let mut subscriber = self.status.connection.clone();
            let (remote_dc, remote_disconnect) = tokio::sync::oneshot::channel();
            let _remote_dc = tokio::spawn(async move {
                loop {
                    let _ = subscriber.changed().await;
                    if subscriber.borrow().is_disconnected() {
                        let _ = remote_dc.send(());
                        break;
                    }
                }
            });

            Ok(VaraStream {
                tnc: self,
                force_disconnect: Some(force_dc),
                remote_disconnect: remote_disconnect,
            })
        } else if self.status.connection.borrow().is_disconnected() {
            Err(color_eyre::eyre::eyre!("failed to connect"))
        } else {
            Err(color_eyre::eyre::eyre!("connection state unexpected"))
        }
    }

    fn pinned_data(self: Pin<&mut Self>) -> Pin<&mut TcpStream> {
        Pin::new(self.project().data)
    }
}

#[derive(Debug)]
#[pin_project::pin_project(PinnedDrop)]
pub struct VaraStream<'a> {
    tnc: &'a mut VaraTnc,
    force_disconnect: Option<tokio::sync::oneshot::Sender<()>>,
    remote_disconnect: tokio::sync::oneshot::Receiver<()>,
}

impl<'a> VaraStream<'a> {
    pub async fn disconnect(self) -> color_eyre::Result<()> {
        self.tnc.send_disconnect().await
    }

    pub async fn abort(self) -> color_eyre::Result<()> {
        self.tnc.send_abort().await
    }
}

#[pin_project::pinned_drop]
impl<'a> PinnedDrop for VaraStream<'a> {
    fn drop(self: Pin<&mut Self>) {
        if !self.tnc.status.connection.borrow().is_disconnected() {
            let this = self.project();
            if let Some(dc) = this.force_disconnect.take() {
                let _ = dc.send(());
            }
            // let _ = this.force_disconnect.send(());
        }
    }
}

impl<'a> AsyncRead for VaraStream<'a> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.project();
        match Pin::new(this.remote_disconnect).poll(cx) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                return Poll::Ready(Ok(()));
            }
        }

        Pin::new(&mut **this.tnc).pinned_data().poll_read(cx, buf)
    }
}

impl<'a> AsyncWrite for VaraStream<'a> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, Error>> {
        let this = self.project();
        match Pin::new(this.remote_disconnect).poll(cx) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "connection closed on remote end")));
            }
        }

        Pin::new(&mut **this.tnc).pinned_data().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let this = self.project();
        match Pin::new(this.remote_disconnect).poll(cx) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "connection closed on remote end")));
            }
        }

        Pin::new(&mut **this.tnc).pinned_data().poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        let this = self.project();
        match Pin::new(this.remote_disconnect).poll(cx) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "connection closed on remote end")));
            }
        }

        Pin::new(&mut **this.tnc).pinned_data().poll_shutdown(cx)
    }

    fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[IoSlice<'_>]) -> Poll<Result<usize, Error>> {
        let this = self.project();
        match Pin::new(this.remote_disconnect).poll(cx) {
            Poll::Pending => {}
            Poll::Ready(_) => {
                return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "connection closed on remote end")));
            }
        }

        Pin::new(&mut **this.tnc).pinned_data().poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.tnc.data.is_write_vectored()
    }
}
