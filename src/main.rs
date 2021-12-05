use std::io::{self, Read, Write};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use plusendi::StationId;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(about, author)]
struct Opt {
    /// The station to connect to
    target: StationId,

    /// My station
    #[structopt(short = "d", long = "de", display_order = 1)]
    my_call: StationId,

    #[structopt(long, default_value = "127.0.0.1")]
    modem_address: std::net::IpAddr,

    #[structopt(long, default_value = "8300")]
    modem_control_port: u16,

    #[structopt(long)]
    modem_data_port: Option<u16>,

    #[structopt(long)]
    rig_control: String,

    #[structopt(long, possible_values(&["4800", "9600", "19200", "38400"]))]
    rig_baud: u32,

    /// Configures internal logging
    #[structopt(short, long, env = "RUST_LOG", default_value = "info", global = true)]
    log: String,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    dotenv::dotenv().ok();
    color_eyre::install()?;

    let opt: Opt = Opt::from_args();

    let env_filter = EnvFilter::try_new(&opt.log).unwrap();
    let fmt_layer = tracing_subscriber::fmt::layer();

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(tracing_error::ErrorLayer::default())
        .init();

    // let mut rig = serialport::new("COM3", 38400).open()?;
    // rig.write_all(b"ID;")?;
    // std::thread::sleep_ms(100);
    // //rig.write_all(b"RX;")?;
    // read_response(&mut rig)?;
    // return Ok(());

    let mut ctl = TcpStream::connect((opt.modem_address, opt.modem_control_port)).await?;
    let modem_data_port = opt.modem_data_port.unwrap_or_else(|| opt.modem_control_port + 1);
    let mut data = TcpStream::connect((opt.modem_address, modem_data_port)).await?;

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(1);
    let (vara_cmd_tx, mut vara_cmd_rx) = tokio::sync::mpsc::channel(1);
    let (rig_tx, rig_rx) = tokio::sync::mpsc::channel(1);

    let mut rig = tokio_serial::SerialStream::open(&tokio_serial::new(opt.rig_control, opt.rig_baud))?;
    #[cfg(unix)]
    rig.set_exclusive(true)?;

    let _thread = tokio::spawn(plusendi::modem::vara::manage_modem_thread(cmd_rx, vara_cmd_tx, ctl));
    let _thread2 = tokio::spawn(plusendi::rig::elecraft::kx3::manage_rig_thread(rig_rx, rig));
    let rig_clone_tx = rig_tx.clone();
    let _thread3 = tokio::spawn(async move {
        while let Some(cmd) = vara_cmd_rx.recv().await {
            tracing::trace!(?cmd, "received automated rig control request");
            let request = match cmd {
                plusendi::modem::vara::TransceiverCommand::Transmit => plusendi::rig::elecraft::kx3::TransmitState::Transmit,
                plusendi::modem::vara::TransceiverCommand::Receive => plusendi::rig::elecraft::kx3::TransmitState::Receive,
            };
            rig_clone_tx.send(plusendi::rig::elecraft::kx3::Command::SetTransmitState(request)).await?;
        }
        tracing::info!("all done with automatic rig control");
        color_eyre::Result::<_, color_eyre::Report>::Ok(())
    });

    rig_tx.send(plusendi::rig::elecraft::kx3::Command::SetTransmitState(plusendi::rig::elecraft::kx3::TransmitState::Transmit)).await?;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    rig_tx.send(plusendi::rig::elecraft::kx3::Command::SetTransmitState(plusendi::rig::elecraft::kx3::TransmitState::Receive)).await?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::SetCall(plusendi::modem::vara::MyCallSigns(opt.my_call.clone(), Vec::new())), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::SetCompression(plusendi::modem::vara::CompressionMode::Text), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Listen(plusendi::modem::vara::ListenMode::Enable), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Connect(plusendi::modem::vara::ConnectCommand { origin: opt.my_call, target: opt.target, path: plusendi::modem::vara::ConnectPath::Direct }), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    tracing::info!("sleep time");
    std::io::stdin().read_line(&mut String::new());

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Disconnect, reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Listen(plusendi::modem::vara::ListenMode::CQ), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    drop(cmd_tx);

    // tokio::time::sleep(std::time::Duration::from_secs(90)).await;

    Ok(())
}

fn read_response(ctl: &mut dyn Read) -> io::Result<()> {
    let mut buffer = [0; 6];
    let count = ctl.read(&mut buffer)?;
    let data = std::str::from_utf8(&buffer[0..count]).expect("ASCII");
    println!("data: {}", data.trim());
    Ok(())
}
