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

use anyhow::Context;
extern crate alloc;

///
#[allow(clippy::module_name_repetitions)]
#[non_exhaustive]
#[derive(Debug, Eq, Clone, Hash, PartialEq)]
pub struct SenderParameters {
    /// ip
    pub relay_target: String,
    /// server name
    pub server_name: String,
    ///
    pub hello_name: String,
    ///
    pub pool_idle_timeout: core::time::Duration,
    ///
    pub pool_max_size: u32,
    ///
    pub pool_min_idle: u32,
    ///
    pub port: u16,
    ///
    pub certificate: Vec<rustls::Certificate>,
    // use_dane: bool,
}

type SenderInner = alloc::sync::Arc<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>>;

///
#[derive(Default)]
pub struct Sender {
    senders: std::sync::RwLock<std::collections::HashMap<SenderParameters, SenderInner>>,
}

impl Sender {
    /// Send a mail to the transport using the given parameters.
    /// Create a new transport if none existing.
    ///
    /// # Errors
    ///
    /// * The inner `RwLock` is poisoned.
    /// * [`lettre::AsyncTransport::send_raw()`] fails.
    #[inline]
    pub async fn send(
        &self,
        params: &SenderParameters,
        envelop: &lettre::address::Envelope,
        message: &[u8],
    ) -> anyhow::Result<lettre::transport::smtp::response::Response> {
        use lettre::AsyncTransport;

        let sender = {
            if !self
                .senders
                .read()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .contains_key(params)
            {
                tracing::trace!(?params, "Key no found for transport with parameters");

                let new_sender = Self::build_sender(params)?;
                let mut writer = self
                    .senders
                    .write()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                writer.insert(params.clone(), new_sender);
            }

            alloc::sync::Arc::clone(
                #[allow(clippy::expect_used)]
                self.senders
                    .read()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?
                    .get(params)
                    .expect("key added right before"),
            )
        };

        sender
            .send_raw(envelop, message)
            .await
            .context("fail to send email")
    }

    fn build_sender(params: &SenderParameters) -> anyhow::Result<SenderInner> {
        tracing::trace!(?params, "Creating a transport");

        let builder = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(
            params.relay_target.clone(),
        )
        .port(params.port)
        .hello_name(lettre::transport::smtp::extension::ClientId::Domain(
            params.hello_name.clone(),
        ))
        .pool_config(
            lettre::transport::smtp::PoolConfig::new()
                .idle_timeout(params.pool_idle_timeout)
                .max_size(params.pool_max_size)
                .min_idle(params.pool_min_idle),
        );

        // NOTE: there is no way to build `lettre::transport::smtp::client::Certificate` from `Vec<rustls::Certificate>`.
        // rustls::Certificate => PEM => lettre::transport::smtp::client::Certificate => rustls::Certificate
        let certs = params
            .certificate
            .iter()
            .map(|c| {
                pem::encode(&pem::Pem {
                    tag: "CERTIFICATE".to_owned(),
                    contents: c.0.clone(),
                })
            })
            .flat_map(|c| c.as_bytes().to_vec())
            .collect::<Vec<_>>();

        let builder = builder.tls(lettre::transport::smtp::client::Tls::Required(
            lettre::transport::smtp::client::TlsParameters::builder(params.server_name.clone())
                .add_root_certificate(lettre::transport::smtp::client::Certificate::from_pem(
                    &certs,
                )?)
                .build()?,
        ));

        // builder.timeout(timeout)

        Ok(alloc::sync::Arc::new(builder.build()))
    }
}
