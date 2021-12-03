use std::io::{self, Read, Write};
use std::net::TcpStream;

fn main() -> color_eyre::Result<()> {
    let mut rig = serialport::new("COM3", 38400).open()?;
    rig.write_all(b"ID;")?;
    std::thread::sleep_ms(100);
    //rig.write_all(b"RX;")?;
    read_response(&mut rig)?;
    return Ok(());

    let mut ctl = TcpStream::connect("127.0.0.1:8300")?;
    let mut data = TcpStream::connect("127.0.0.1:8301")?;

    ctl.write_all(b"MYCALL KC1GSL\r")?;
    read_response(&mut ctl)?;

    ctl.write_all(b"CONNECT KC1GSL KW1U\r")?;
    read_response(&mut ctl)?;

    for _ in 0..40 {
        read_response(&mut ctl)?;
    }

    ctl.write_all(b"DISCONNECT")?;

    Ok(())
}

fn read_response(ctl: &mut dyn Read) -> io::Result<()> {
    let mut buffer = [0; 6];
    let count = ctl.read(&mut buffer)?;
    let data = std::str::from_utf8(&buffer[0..count]).expect("ASCII");
    println!("data: {}", data.trim());
    Ok(())
}
