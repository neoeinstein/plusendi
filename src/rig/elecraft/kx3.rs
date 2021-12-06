use std::fmt;
use std::fmt::Write;
use nom::{AsBytes, IResult};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    SetTransmitState(TransmitState)
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetTransmitState(x) => fmt::Display::fmt(x, f),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransmitState {
    Receive,
    Transmit,
}

impl fmt::Display for TransmitState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::Receive => "RX",
            Self::Transmit => "TX",
        };

        f.write_str(code)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update<'a> {
    Filler(&'a str)
}

fn line(data: &[u8]) -> IResult<&[u8], &[u8]> {
    nom::sequence::terminated(nom::bytes::streaming::take_until1(";"), nom::bytes::streaming::tag(";"))(data)
}

#[tracing::instrument(skip(rx, stream), err)]
pub async fn manage_rig_thread<D: AsyncRead + AsyncWrite + Unpin + 'static>(mut rx: mpsc::Receiver<Command>, /*tx: broadcast::Sender<Update<'static>>, */mut stream: D) -> color_eyre::Result<()> {
    let mut cmd_buffer = String::with_capacity(32);
    let mut upd_buffer = bytes::BytesMut::with_capacity(32);
    let mut command_active = true;

    while command_active {
        tokio::select!(
            recv = rx.recv() => {
                if let Some(command) = recv {
                    cmd_buffer.clear();
                    write!(&mut cmd_buffer, "{};", command).unwrap();
                    tracing::trace!(command = cmd_buffer.as_str(), "sending command");
                    stream.write_all(cmd_buffer.as_bytes()).await?;
                } else {
                    command_active = false
                }
            },
            result = stream.read_buf(&mut upd_buffer) => {
                match result {
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
                                //
                                // match nom::combinator::all_consuming(update)(line).map_err(|e| e.map_input(|i| String::from_utf8(i.into()).unwrap())).finish() {
                                //     Ok((_ , update)) => {
                                //         tracing::debug!(?update, "received update");
                                //         match update {
                                //             Update::CommandResult(result) => {
                                //                 if result == CommandResult::Ok {
                                //                     to_acknowledge.push(Ok(()))
                                //                 } else {
                                //                     to_acknowledge.push(Err(()))
                                //                 }
                                //             }
                                //             Update::Heartbeat => {
                                //                 tracing::debug!("received heartbeat");
                                //             }
                                //             Update::TransceiverControl(control) => {
                                //                 //tracing::debug!("")
                                //             }
                                //             _ => {}
                                //         }
                                //     }
                                //     Err(err) => {
                                //         return Err(err.into());
                                //     }
                                // }
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
                    upd_buffer = new;
                    tracing::trace!(bytes = upd_buffer.len(), "retained incomplete parts");
                }
            }
        );
    }
    tracing::info!("exiting command loop");
    Ok(())
}
