use aliri_braid::braid;
use std::collections::hash_set::HashSet;

pub mod modem;
mod types;

pub use modem::Modem;
pub use types::{StationId, StationIdRef};

#[derive(Debug, PartialEq, Eq)]
pub struct Traffic {
    pub header: TrafficHeader,
    pub destination: Destination,
    pub body: String,
    pub signature: Signature,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Destination {
    pub addressee: String,
    pub station: Option<StationId>,
    pub address: Vec<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub op_note: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Signature {
    pub signed_by: String,
    pub op_note: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrafficHeader {
    pub service: ServiceType,
    pub number: u16,
    pub traffic_type: TrafficType,
    pub precedence: Precedence,
    pub handling: Handling,
    pub originator: StationId,
    pub check: Check,
    pub origin: String,
    pub time_filed: Option<String>,
    pub date: String,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum ServiceType {
    Normal,
    Service,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum TrafficType {
    Normal,
    Exercise,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Routine,
    Welfare,
    Priority,
    Emergency,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Handling {
    directives: HashSet<HandlingDirective>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum HandlingDirective {
    LandlineCollect {
        distance: u16,
    },
    DeliverWithin {
        hours: u8,
    },
    ReportDelivery,
    TraceRelayAndDelivery,
    RequestReply,
    HoldUntil {
        date: String,
    },
    CancelIfFeeRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    content: ContentType,
    count: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentType {
    Standard,
    Arl,
}
