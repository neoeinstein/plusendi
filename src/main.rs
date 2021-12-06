use std::num::NonZeroU16;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use plusendi::StationId;
use structopt::StructOpt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

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
    modem_control_port: NonZeroU16,

    #[structopt(long)]
    modem_data_port: Option<NonZeroU16>,

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

    let mut builder = plusendi::modem::vara::VaraTnc::builder();

    builder.host(opt.modem_address)
        .control_port(opt.modem_control_port);

    if let Some(port) = opt.modem_data_port {
        builder.data_port(port);
    }

    let mut tnc = builder.build().await?;

    tnc.send_callsign(opt.my_call.clone()).await?;
    tnc.send_compression(plusendi::modem::vara::CompressionMode::Text).await?;
    tnc.send_bandwidth(plusendi::modem::vara::BandwidthMode::Wide).await?;
    let mut transceiver_cmd = tnc.subscribe_rig_command();

    let (rig_tx, rig_rx) = tokio::sync::mpsc::channel(1);

    let mut rig = tokio_serial::SerialStream::open(&tokio_serial::new(opt.rig_control, opt.rig_baud))?;
    #[cfg(unix)]
        rig.set_exclusive(true)?;

    let _thread2 = tokio::spawn(plusendi::rig::elecraft::kx3::manage_rig_thread(rig_rx, rig));
    let _thread3 = tokio::spawn(async move {
        while let Ok(()) = transceiver_cmd.changed().await {
            let request = {
                let cmd = *transceiver_cmd.borrow();
                tracing::trace!(?cmd, "received automated rig control request");
                match cmd {
                    plusendi::modem::vara::TransceiverCommand::Transmit => plusendi::rig::elecraft::kx3::TransmitState::Transmit,
                    plusendi::modem::vara::TransceiverCommand::Receive => plusendi::rig::elecraft::kx3::TransmitState::Receive,
                }
            };
            rig_tx.send(plusendi::rig::elecraft::kx3::Command::SetTransmitState(request)).await?;
        }
        tracing::info!("all done with automatic rig control");
        color_eyre::Result::<_, color_eyre::Report>::Ok(())
    });

    let mut vara_stream = tnc.connect(opt.my_call, opt.target).await?;

    tracing::info!("sleep time");
    let mut read = bytes::BytesMut::new();

    fn line(data: &[u8]) -> nom::IResult<&[u8], &[u8]> {
        nom::sequence::terminated(nom::bytes::streaming::take_until1("\r"), nom::bytes::streaming::tag("\r"))(data)
    }

    'out: loop {
        vara_stream.read_buf(&mut read).await?;
        let retain_after = {
            let mut data = &read[..];
            while data.len() > 0 {
                match line(&data) {
                    Ok((remaining, line)) => {
                        tracing::trace!(line = std::str::from_utf8(line).unwrap(), remaining = std::str::from_utf8(remaining).unwrap(), "received complete line");
                        data = remaining;
                        println!("{}", String::from_utf8_lossy(line));
                        if line.ends_with(&[b'>']) {
                            break 'out;
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
            read.len() - data.len()
        };
        if retain_after == read.len() {
            read.clear();
        } else if retain_after > 0 {
            let new = read.split_off(retain_after);
            read = new;
            tracing::trace!(bytes = read.len(), "retained incomplete parts");
        }
    }
    // loop {
    //     match std::io::stdin().read_line(&mut to_send) {
    //         Ok(0) => break,
    //         Ok(b) => tracing::trace!(bytes = b, "sending bytes"),
    //         Err(error) => return Err(error.into()),
    //     }
    //     // data.write_all(to_send.as_bytes()).await?;
    //     // data.write_all(b"\r").await?;
    // }
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    input.read_line(&mut String::new()).await?;
    let ident = format!("{}-{}", env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"));
    let to_be_sent = format!("[{}-B2FWIHJM$]\rFF\r", ident);
    vara_stream.write_all(to_be_sent.as_bytes()).await?;
    read.clear();
    loop {
        let count = vara_stream.read_buf(&mut read).await?;
        if count == 0 {
            break;
        }
        let retain_after = {
            let mut data = &read[..];
            while data.len() > 0 {
                match line(&data) {
                    Ok((remaining, line)) => {
                        tracing::trace!(line = std::str::from_utf8(line).unwrap(), remaining = std::str::from_utf8(remaining).unwrap(), "received complete line");
                        data = remaining;
                        println!("{}", String::from_utf8_lossy(line));
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
            read.len() - data.len()
        };
        if retain_after == read.len() {
            read.clear();
        } else if retain_after > 0 {
            let new = read.split_off(retain_after);
            read = new;
            tracing::trace!(bytes = read.len(), "retained incomplete parts");
        }
    }

    // data.write_all(b"[Plusendi-0.0.1-B2FWIHJM$]\rFF\r");

    tracing::info!("sleep time");
    input.read_line(&mut String::new()).await?;

    // tokio::time::sleep(std::time::Duration::from_secs(90)).await;

    Ok(())
}
