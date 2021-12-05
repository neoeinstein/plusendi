use crate::StationIdRef;

pub mod vara;

pub trait Modem<'a> {
    type Connection;
    type ConnectionError: std::error::Error + Send + Sync + 'static;

    fn connect(&'a mut self, station: &StationIdRef) -> Result<Self::Connection, Self::ConnectionError>;
}
