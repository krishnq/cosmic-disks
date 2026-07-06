// SPDX-License-Identifier: GPL-3.0

//! Subscribes to udisks2 D-Bus signals and triggers a drive refresh on change.
//!
//! We match on any signal from the `org.freedesktop.UDisks2` sender, which
//! covers ObjectAdded, ObjectRemoved, and PropertiesChanged (mount-state changes).
//! A 500 ms debounce window collapses bursts into a single refresh.

use cosmic::iced::futures::SinkExt;
use futures_util::StreamExt;
use std::time::Duration;
use zbus::message::Type as MessageType;
use zbus::{Connection, MatchRule, MessageStream};

use crate::message::Message;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) fn subscription() -> cosmic::iced::Subscription<Message> {
    cosmic::iced::Subscription::run(watch_impl)
}

fn watch_impl() -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::stream::channel(4, |mut tx| async move {
        loop {
            if let Err(e) = run_watch(&mut tx).await {
                log::warn!("udisks2 watcher error: {e}, retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    })
}

async fn run_watch(
    tx: &mut cosmic::iced::futures::channel::mpsc::Sender<Message>,
) -> Result<(), BoxError> {
    let conn = Connection::system().await?;

    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender("org.freedesktop.UDisks2")?
        .build();

    let mut stream = MessageStream::for_match_rule(rule, &conn, Some(32)).await?;

    loop {
        // An idle await here is fine — no signals just means nothing changed.
        // Stream end (None) means the connection dropped; return so the outer
        // loop reconnects.
        if stream.next().await.is_none() {
            break;
        }

        // Debounce: drain any further signals arriving within 500 ms.
        while let Ok(Some(_)) =
            tokio::time::timeout(Duration::from_millis(500), stream.next()).await
        {}

        if tx.send(Message::RefreshDisks).await.is_err() {
            break;
        }
    }

    Ok(())
}
