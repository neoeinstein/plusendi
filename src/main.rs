use std::io::{self, Read, Write};
use tokio::net::TcpStream;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    dotenv::dotenv().ok();
    color_eyre::install()?;

    // let opt: Opt = Opt::from_args();

    //let env_filter = EnvFilter::try_new(&opt.log).unwrap();
    let env_filter = EnvFilter::from_default_env();
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

    let mut ctl = TcpStream::connect("127.0.0.1:8300").await?;
    // let mut data = TcpStream::connect("127.0.0.1:8301")?;

    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(1);

    let _thread = tokio::spawn(plusendi::modem::vara::manage_modem_thread(cmd_rx, ctl));

    let my_call = plusendi::StationId::new("KX1XXX")?;
    let other_call = plusendi::StationId::new("KX1X")?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::SetCall(plusendi::modem::vara::MyCallSigns(my_call.clone(), Vec::new())), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::SetCompression(plusendi::modem::vara::CompressionMode::Text), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Listen(plusendi::modem::vara::ListenMode::Enable), reply_tx)).await?;
    reply_rx.await?.map_err(|_| color_eyre::eyre::eyre!("command went wrong"))?;

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    cmd_tx.send((plusendi::modem::vara::Command::Connect(plusendi::modem::vara::ConnectCommand { origin: my_call, target: other_call, path: plusendi::modem::vara::ConnectPath::Direct }), reply_tx)).await?;
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
