/*
 * vSMTP mail transfer agent
 * Copyright (C) 2022 viridIT SAS
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or any later version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
 * FOR A PARTICULAR PURPOSE.  See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License along with
 * this program. If not, see https://www.gnu.org/licenses/.
 *
*/
use crate::ProcessMessage;
use anyhow::Context;
use time::ext::NumericalDuration;
use vqueue::{GenericQueueManager, QueueID};
use vsmtp_common::transfer::EmailTransferStatus;
use vsmtp_config::{Config, DnsResolvers};
use vsmtp_delivery::{split_and_sort_and_send, Sender, SenderOutcome};

pub async fn flush_deferred_queue<Q: GenericQueueManager + Sized + 'static>(
    config: std::sync::Arc<Config>,
    resolvers: std::sync::Arc<DnsResolvers>,
    queue_manager: std::sync::Arc<Q>,
    sender: std::sync::Arc<Sender>,
    flushing_at: time::OffsetDateTime,
) {
    let queued = match queue_manager.list(&QueueID::Deferred).await {
        Ok(queued) => queued,
        Err(error) => {
            tracing::error!(%error, "Listing deferred queue failure.");
            return;
        }
    };

    for i in queued {
        let message_uuid = match i.map(|i| uuid::Uuid::parse_str(&i)) {
            Ok(Ok(message_uuid)) => message_uuid,
            Ok(Err(error)) => {
                tracing::error!(%error, "Invalid message id in deferred queue.");
                continue;
            }
            Err(error) => {
                tracing::error!(%error, "Deferred message id missing.");
                continue;
            }
        };

        if let Err(error) = handle_one_in_deferred_queue(
            config.clone(),
            resolvers.clone(),
            queue_manager.clone(),
            ProcessMessage {
                message_uuid,
                delegated: false,
            },
            sender.clone(),
            flushing_at,
        )
        .await
        {
            tracing::error!(%error, "Flushing deferred queue failure.");
        }
    }
}

#[tracing::instrument(name = "deferred", skip_all, err, fields(uuid = %process_message.message_uuid))]
async fn handle_one_in_deferred_queue<Q: GenericQueueManager + Sized + 'static>(
    config: std::sync::Arc<Config>,
    resolvers: std::sync::Arc<DnsResolvers>,
    queue_manager: std::sync::Arc<Q>,
    process_message: ProcessMessage,
    sender: std::sync::Arc<Sender>,
    flushing_at: time::OffsetDateTime,
) -> anyhow::Result<()> {
    tracing::debug!("Processing email.");

    let mut ctx = queue_manager
        .get_ctx(&QueueID::Deferred, &process_message.message_uuid)
        .await?;

    let last_error = ctx
        .rcpt_to
        .forward_paths
        .iter()
        .filter_map(|i| match &i.email_status {
            EmailTransferStatus::HeldBack { errors } => errors.last().map(|e| e.timestamp),
            _ => None,
        })
        .min();

    let held_back_count = ctx
        .rcpt_to
        .forward_paths
        .iter()
        .filter(|i| matches!(i.email_status, EmailTransferStatus::HeldBack { .. }))
        .count() as i64;

    match last_error {
        Some(last_error)
            // last error + (error_count * 5min)
            if last_error
                .checked_add(held_back_count.seconds() * 60 * 5)
                .unwrap()
                > flushing_at =>
        {
            tracing::debug!("Email is not ready to be flushed.");
            return Ok(());
        }
        _ => {}
    }

    let msg = queue_manager.get_msg(&process_message.message_uuid).await?;

    match split_and_sort_and_send(&config, &mut ctx, &msg, resolvers, sender).await {
        SenderOutcome::MoveToDead => queue_manager
            .move_to(&QueueID::Deferred, &QueueID::Dead, &ctx)
            .await
            .with_context(|| {
                format!(
                    "cannot move file from `{}` to `{}`",
                    QueueID::Deferred,
                    QueueID::Dead
                )
            }),
        SenderOutcome::MoveToDeferred => queue_manager
            .write_ctx(&QueueID::Deferred, &ctx)
            .await
            .with_context(|| format!("failed to update context in `{}`", QueueID::Deferred)),
        SenderOutcome::RemoveFromDisk => {
            queue_manager
                .remove_both(&QueueID::Deferred, &process_message.message_uuid)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vsmtp_common::{rcpt::Rcpt, Address};
    use vsmtp_test::config::{local_ctx, local_msg, local_test};

    #[tokio::test]
    async fn move_to_deferred() {
        let config = std::sync::Arc::new(local_test());
        let queue_manager =
            <vqueue::temp::QueueManager as vqueue::GenericQueueManager>::init(config.clone())
                .unwrap();

        let mut ctx = local_ctx();
        let message_uuid = uuid::Uuid::new_v4();
        ctx.mail_from.message_uuid = message_uuid;
        ctx.rcpt_to.forward_paths.push(Rcpt::new(
            <Address as std::str::FromStr>::from_str("test@localhost").unwrap(),
        ));

        queue_manager
            .write_both(&QueueID::Deferred, &ctx, &local_msg())
            .await
            .unwrap();

        let resolvers = std::sync::Arc::new(DnsResolvers::from_config(&config).unwrap());
        let sender = std::sync::Arc::new(Sender::default());

        handle_one_in_deferred_queue(
            config.clone(),
            resolvers,
            queue_manager.clone(),
            ProcessMessage {
                message_uuid,
                delegated: false,
            },
            sender,
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .await
        .unwrap();

        queue_manager
            .get_ctx(&QueueID::Deliver, &message_uuid)
            .await
            .unwrap_err();
        queue_manager
            .get_ctx(&QueueID::Dead, &message_uuid)
            .await
            .unwrap_err();

        queue_manager
            .get_ctx(&QueueID::Deferred, &message_uuid)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn move_to_dead() {
        let config = std::sync::Arc::new(local_test());
        let queue_manager =
            <vqueue::temp::QueueManager as vqueue::GenericQueueManager>::init(config.clone())
                .unwrap();

        let mut ctx = local_ctx();
        let message_uuid = uuid::Uuid::new_v4();
        ctx.mail_from.message_uuid = message_uuid;

        queue_manager
            .write_both(&QueueID::Deferred, &ctx, &local_msg())
            .await
            .unwrap();
        let resolvers = std::sync::Arc::new(DnsResolvers::from_config(&config).unwrap());
        let sender = std::sync::Arc::new(Sender::default());

        handle_one_in_deferred_queue(
            config.clone(),
            resolvers,
            queue_manager.clone(),
            ProcessMessage {
                message_uuid,
                delegated: false,
            },
            sender,
            time::OffsetDateTime::UNIX_EPOCH,
        )
        .await
        .unwrap();

        queue_manager
            .get_ctx(&QueueID::Deferred, &message_uuid)
            .await
            .unwrap_err();

        queue_manager
            .get_ctx(&QueueID::Dead, &message_uuid)
            .await
            .unwrap();
    }
}
