//! Short-lived, single-use tickets for opening the browser WebSocket.
//!
//! Browsers cannot send an `Authorization` header on a WebSocket upgrade, and a
//! session token in a URL would end up in logs. A session instead asks for a ticket
//! over HTTP and presents it once, within `TICKET_TTL`.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::auth::CurrentUser;

pub const TICKET_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Tickets {
    issued: Mutex<HashMap<String, (CurrentUser, Instant)>>,
}

impl Tickets {
    /// Issues a ticket for `user`. Returns the ticket and its expiry time.
    pub fn issue(&self, user: CurrentUser) -> (String, DateTime<Utc>) {
        let ticket = format!("wst_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = Instant::now();
        let mut issued = self.issued.lock().expect("tickets lock poisoned");
        issued.retain(|_, (_, expires)| *expires > now);
        issued.insert(ticket.clone(), (user, now + TICKET_TTL));
        let expires_at = Utc::now() + TimeDelta::seconds(TICKET_TTL.as_secs() as i64);
        (ticket, expires_at)
    }

    /// Consumes a ticket. It cannot be used again, even if it has expired.
    pub fn redeem(&self, ticket: &str) -> Option<CurrentUser> {
        let (user, expires) = self
            .issued
            .lock()
            .expect("tickets lock poisoned")
            .remove(ticket)?;
        (Instant::now() < expires).then_some(user)
    }
}
