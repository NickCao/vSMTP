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

use crate::{
    config::field::{
        FieldApp, FieldAppLogs, FieldAppVSL, FieldQueueDelivery, FieldQueueWorking, FieldServer,
        FieldServerDNS, FieldServerInterfaces, FieldServerLogs, FieldServerQueues, FieldServerSMTP,
        FieldServerSMTPAuth, FieldServerSMTPError, FieldServerSMTPTimeoutClient, FieldServerSystem,
        FieldServerSystemThreadPool, FieldServerTls, FieldServerVirtual, ResolverOptsWrapper,
        SyslogSocket,
    },
    Config,
};
use vsmtp_common::{auth::Mechanism, collection, CodeID, Reply, ReplyCode};

impl Default for Config {
    fn default() -> Self {
        let current_version =
            semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("valid semver");
        Self {
            version_requirement: semver::VersionReq::from_iter([
                semver::Comparator {
                    op: semver::Op::GreaterEq,
                    major: current_version.major,
                    minor: Some(current_version.minor),
                    patch: Some(current_version.patch),
                    pre: current_version.pre,
                },
                semver::Comparator {
                    op: semver::Op::Less,
                    major: current_version.major + 1,
                    minor: Some(0),
                    patch: Some(0),
                    pre: semver::Prerelease::EMPTY,
                },
            ]),
            server: FieldServer::default(),
            app: FieldApp::default(),
            path: None,
        }
    }
}

impl Config {
    /// This function is primarily used to inject a config structure into vsl.
    ///
    /// Context: groups & users MUST be initialized when creating a default configuration.
    /// The configuration COULD be changed in a `vsmtp.vsl` or `config.vsl` script.
    /// But rust does not know that in advance, thus, even tough the user does not
    /// want to use the 'vsmtp' user by default, vsmtp will try to get that user
    /// when creating a default config. This leads to users that MUST create a 'vsmtp'
    /// user, even tough they want to change it in the configuration.
    ///
    /// We could also wrap the user & group configuration variable into an enum, but that will lead
    /// either to a lot of match patters to check if they are set or not, or simply more
    /// unwrap because we know that after the config has been loaded that it is correct.
    #[must_use]
    pub(crate) fn default_with_current_user_and_group() -> Self {
        let current_version =
            semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("valid semver");
        Self {
            version_requirement: semver::VersionReq::from_iter([
                semver::Comparator {
                    op: semver::Op::GreaterEq,
                    major: current_version.major,
                    minor: Some(current_version.minor),
                    patch: Some(current_version.patch),
                    pre: current_version.pre,
                },
                semver::Comparator {
                    op: semver::Op::Less,
                    major: current_version.major + 1,
                    minor: Some(0),
                    patch: Some(0),
                    pre: semver::Prerelease::EMPTY,
                },
            ]),
            server: FieldServer {
                // NOTE: Dirty fix to prevent vsmtp 'default user not found' error message
                //       when injecting a default config instance in vsl config.
                system: FieldServerSystem {
                    user: {
                        let uid = users::get_current_uid();

                        users::get_user_by_uid(uid).expect("current uid must be valid")
                    },
                    group: {
                        let gid = users::get_current_gid();

                        users::get_group_by_gid(gid).expect("current gid must be valid")
                    },
                    group_local: None,
                    thread_pool: FieldServerSystemThreadPool::default(),
                },
                // All of this is necessary since `FieldServer` implements a custom
                // default function instead of using the derivative macro.
                name: FieldServer::hostname(),
                client_count_max: FieldServer::default_client_count_max(),
                message_size_limit: FieldServer::default_message_size_limit(),
                interfaces: FieldServerInterfaces::default(),
                logs: FieldServerLogs::default(),
                queues: FieldServerQueues::default(),
                tls: None,
                smtp: FieldServerSMTP::default(),
                dns: FieldServerDNS::default(),
                r#virtual: std::collections::BTreeMap::default(),
            },
            app: FieldApp::default(),
            path: None,
        }
    }
}

impl Default for FieldServer {
    fn default() -> Self {
        Self {
            name: Self::hostname(),
            client_count_max: Self::default_client_count_max(),
            message_size_limit: Self::default_message_size_limit(),
            system: FieldServerSystem::default(),
            interfaces: FieldServerInterfaces::default(),
            logs: FieldServerLogs::default(),
            queues: FieldServerQueues::default(),
            tls: None,
            smtp: FieldServerSMTP::default(),
            dns: FieldServerDNS::default(),
            r#virtual: std::collections::BTreeMap::default(),
        }
    }
}

impl FieldServer {
    pub(crate) fn hostname() -> String {
        hostname::get().unwrap().to_str().unwrap().to_string()
    }

    pub(crate) const fn default_client_count_max() -> i64 {
        16
    }

    pub(crate) const fn default_message_size_limit() -> usize {
        10_000_000
    }
}

impl Default for FieldServerSystem {
    fn default() -> Self {
        Self {
            user: Self::default_user(),
            group: Self::default_group(),
            group_local: None,
            thread_pool: FieldServerSystemThreadPool::default(),
        }
    }
}

impl FieldServerSystem {
    pub(crate) fn default_user() -> users::User {
        users::get_user_by_name(match option_env!("CI") {
            Some(_) => "root",
            None => "vsmtp",
        })
        .expect("default user 'vsmtp' not found.")
    }

    pub(crate) fn default_group() -> users::Group {
        users::get_group_by_name(match option_env!("CI") {
            Some(_) => "root",
            None => "vsmtp",
        })
        .expect("default group 'vsmtp' not found.")
    }
}

impl Default for FieldServerSystemThreadPool {
    fn default() -> Self {
        Self {
            receiver: Self::default_receiver(),
            processing: Self::default_processing(),
            delivery: Self::default_delivery(),
        }
    }
}

impl FieldServerSystemThreadPool {
    pub(crate) const fn default_receiver() -> usize {
        6
    }

    pub(crate) const fn default_processing() -> usize {
        6
    }

    pub(crate) const fn default_delivery() -> usize {
        6
    }
}

impl Default for FieldServerInterfaces {
    fn default() -> Self {
        Self::ipv4_localhost()
    }
}

impl FieldServerInterfaces {
    pub(crate) fn ipv4_localhost() -> Self {
        Self {
            addr: vec!["127.0.0.1:25".parse().expect("valid")],
            addr_submission: vec!["127.0.0.1:587".parse().expect("valid")],
            addr_submissions: vec!["127.0.0.1:465".parse().expect("valid")],
        }
    }
}

impl Default for FieldServerLogs {
    fn default() -> Self {
        Self {
            filename: Self::default_filename(),
            level: Self::default_level(),
            system: None,
        }
    }
}

impl FieldServerLogs {
    pub(crate) fn default_filename() -> std::path::PathBuf {
        "/var/log/vsmtp/vsmtp.log".into()
    }

    pub(crate) fn default_level() -> Vec<tracing_subscriber::filter::Directive> {
        vec!["warn".parse().expect("hardcoded value is valid")]
    }
}

impl SyslogSocket {
    pub(crate) fn default_udp_local() -> std::net::SocketAddr {
        "127.0.0.1:0".parse().expect("valid")
    }

    pub(crate) fn default_udp_server() -> std::net::SocketAddr {
        "127.0.0.1:514".parse().expect("valid")
    }

    pub(crate) fn default_tcp_server() -> std::net::SocketAddr {
        "127.0.0.1:601".parse().expect("valid")
    }
}

impl Default for SyslogSocket {
    fn default() -> Self {
        Self::Unix { path: None }
    }
}

impl FieldServerTls {
    pub(crate) fn default_cipher_suite() -> Vec<vsmtp_common::CipherSuite> {
        [
            // TLS1.3 suites
            rustls::CipherSuite::TLS13_AES_256_GCM_SHA384,
            rustls::CipherSuite::TLS13_AES_128_GCM_SHA256,
            rustls::CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
            // TLS1.2 suites
            rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
            rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
            rustls::CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
            rustls::CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
            rustls::CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
            rustls::CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
        ]
        .into_iter()
        .map(vsmtp_common::CipherSuite)
        .collect::<Vec<_>>()
    }

    pub(crate) const fn default_handshake_timeout() -> std::time::Duration {
        std::time::Duration::from_secs(1)
    }
}

impl Default for FieldServerQueues {
    fn default() -> Self {
        Self {
            dirpath: Self::default_dirpath(),
            working: FieldQueueWorking::default(),
            delivery: FieldQueueDelivery::default(),
        }
    }
}

impl FieldServerQueues {
    pub(crate) fn default_dirpath() -> std::path::PathBuf {
        "/var/spool/vsmtp".into()
    }
}

impl Default for FieldQueueWorking {
    fn default() -> Self {
        Self {
            channel_size: Self::default_channel_size(),
        }
    }
}

impl FieldQueueWorking {
    pub(crate) const fn default_channel_size() -> usize {
        32
    }
}

impl Default for FieldQueueDelivery {
    fn default() -> Self {
        Self {
            channel_size: Self::default_channel_size(),
            deferred_retry_max: Self::default_deferred_retry_max(),
            deferred_retry_period: Self::default_deferred_retry_period(),
        }
    }
}

impl FieldQueueDelivery {
    pub(crate) const fn default_channel_size() -> usize {
        32
    }

    pub(crate) const fn default_deferred_retry_max() -> usize {
        100
    }

    pub(crate) const fn default_deferred_retry_period() -> std::time::Duration {
        std::time::Duration::from_secs(300)
    }
}

impl FieldServerVirtual {
    pub(crate) fn default_json() -> anyhow::Result<rhai::Map> {
        Ok(rhai::Engine::new().parse_json(serde_json::to_string(&Self::default())?, true)?)
    }
}

impl Default for FieldServerSMTPAuth {
    fn default() -> Self {
        Self {
            enable_dangerous_mechanism_in_clair: Self::default_enable_dangerous_mechanism_in_clair(
            ),
            mechanisms: Self::default_mechanisms(),
            attempt_count_max: Self::default_attempt_count_max(),
        }
    }
}

impl FieldServerSMTPAuth {
    pub(crate) const fn default_enable_dangerous_mechanism_in_clair() -> bool {
        false
    }

    /// Return all the supported SASL mechanisms
    #[must_use]
    pub fn default_mechanisms() -> Vec<Mechanism> {
        vec![Mechanism::Plain, Mechanism::Login, Mechanism::CramMd5]
    }

    pub(crate) const fn default_attempt_count_max() -> i64 {
        -1
    }
}

impl Default for FieldServerSMTP {
    fn default() -> Self {
        Self {
            rcpt_count_max: Self::default_rcpt_count_max(),
            error: FieldServerSMTPError::default(),
            timeout_client: FieldServerSMTPTimeoutClient::default(),
            codes: Self::default_smtp_codes(),
            auth: None,
        }
    }
}

impl FieldServerSMTP {
    pub(crate) const fn default_rcpt_count_max() -> usize {
        1000
    }

    // TODO: should be const and compile time checked
    pub(crate) fn default_smtp_codes() -> std::collections::BTreeMap<CodeID, Reply> {
        let codes: std::collections::BTreeMap<CodeID, Reply> = collection! {
            CodeID::Greetings => Reply::new(
                ReplyCode::Code{ code: 220 }, "{name} Service ready\r\n"
            ),
            CodeID::Help => Reply::new(
                ReplyCode::Code{ code: 214 }, "joining us https://viridit.com/support\r\n"
            ),
            CodeID::Closing => Reply::new(
                ReplyCode::Code{ code: 221 }, "Service closing transmission channel\r\n"
            ),
            CodeID::Helo => Reply::new(
                ReplyCode::Code{ code: 250 }, "Ok\r\n"
            ),
            // CodeID::EhloPain => Reply::new(
            //     ReplyCode::Code{ code: 200 }, ""
            // ),
            // CodeID::EhloSecured => Reply::new(
            //     ReplyCode::Code{ code: 200 }, ""
            // ),
            CodeID::DataStart => Reply::new(
                ReplyCode::Code{ code: 354 }, "Start mail input; end with <CRLF>.<CRLF>\r\n"
            ),
            CodeID::Ok => Reply::new(
                ReplyCode::Code{ code: 250 }, "Ok\r\n"
            ),
            CodeID::Failure => Reply::new(
                ReplyCode::Code{ code: 451 }, "Requested action aborted: local error in processing\r\n"
            ),
            CodeID::Denied => Reply::new(
                ReplyCode::Code{ code: 554 }, "permanent problems with the remote server\r\n"
            ),
            CodeID::UnrecognizedCommand => Reply::new(
                ReplyCode::Code{ code: 500 }, "Syntax error command unrecognized\r\n"
            ),
            CodeID::SyntaxErrorParams => Reply::new(
                ReplyCode::Code{ code: 501 }, "Syntax error in parameters or arguments\r\n"
            ),
            CodeID::ParameterUnimplemented => Reply::new(
                ReplyCode::Code{ code: 504 }, "Command parameter not implemented\r\n"
            ),
            CodeID::Unimplemented => Reply::new(
                ReplyCode::Code{ code: 502 }, "Command not implemented\r\n"
            ),
            CodeID::BadSequence => Reply::new(
                ReplyCode::Code{ code: 503 }, "Bad sequence of commands\r\n"
            ),
            CodeID::MessageSizeExceeded => Reply::new(
                ReplyCode::Enhanced { code: 552, enhanced: "4.3.1".to_string() }, "Message size exceeds fixed maximum message size\r\n"
            ),
            CodeID::TlsGoAhead => Reply::new(
                ReplyCode::Code{ code: 220 }, "TLS go ahead\r\n"
            ),
            CodeID::TlsNotAvailable => Reply::new(
                ReplyCode::Code{ code: 454 }, "TLS not available due to temporary reason\r\n"
            ),
            CodeID::AlreadyUnderTLS => Reply::new(
                ReplyCode::Enhanced{ code: 554, enhanced: "5.5.1".to_string() }, "Error: TLS already active\r\n"
            ),
            CodeID::AuthSucceeded => Reply::new(
                ReplyCode::Enhanced{ code: 235, enhanced: "2.7.0".to_string() }, "Authentication succeeded\r\n"
            ),
            CodeID::AuthMechNotSupported => Reply::new(
                ReplyCode::Enhanced{ code: 504, enhanced: "5.5.4".to_string() }, "Mechanism is not supported\r\n"
            ),
            CodeID::AuthClientMustNotStart => Reply::new(
                ReplyCode::Enhanced{ code: 501, enhanced: "5.7.0".to_string() }, "Client must not start with this mechanism\r\n"
            ),
            CodeID::AuthMechanismMustBeEncrypted => Reply::new(
                ReplyCode::Enhanced{ code: 538, enhanced: "5.7.11".to_string() },
                    "Encryption required for requested authentication mechanism\r\n"
            ),
            CodeID::AuthInvalidCredentials => Reply::new(
                ReplyCode::Enhanced{ code: 535, enhanced: "5.7.8".to_string() }, "Authentication credentials invalid\r\n"
            ),
            CodeID::AuthClientCanceled => Reply::new(
                ReplyCode::Code{ code: 501 }, "Authentication canceled by client\r\n"
            ),
            CodeID::AuthErrorDecode64 => Reply::new(
                ReplyCode::Enhanced{ code: 501, enhanced: "5.5.2".to_string() }, "Invalid, not base64\r\n"
            ),
            CodeID::AuthTempError => Reply::new(
                ReplyCode::Enhanced{ code: 454, enhanced: "4.7.0".to_string() }, "Temporary authentication failure\r\n"
            ),
            CodeID::ConnectionMaxReached => Reply::new(
                ReplyCode::Code{ code: 554 }, "Cannot process connection, closing\r\n"
            ),
            CodeID::TooManyError => Reply::new(
                ReplyCode::Code{ code: 451 }, "Too many errors from the client\r\n"
            ),
            CodeID::Timeout => Reply::new(
                ReplyCode::Code{ code: 451 }, "Timeout - closing connection\r\n"
            ),
            CodeID::TooManyRecipients => Reply::new(
                ReplyCode::Code{ code: 452 }, "Requested action not taken: too many recipients\r\n"
            ),
        };

        assert!(
            <CodeID as strum::IntoEnumIterator>::iter()
                // exclude these codes because they are generated by the [Config::ensure] and vsl.
                .filter(|i| ![CodeID::EhloPain, CodeID::EhloSecured,].contains(i))
                .all(|i| codes.contains_key(&i)),
            "default SMTPReplyCode are ill-formed "
        );

        codes
    }
}

impl Default for FieldServerDNS {
    fn default() -> Self {
        Self::System
    }
}

impl Default for ResolverOptsWrapper {
    fn default() -> Self {
        Self {
            timeout: Self::default_timeout(),
            attempts: Self::default_attempts(),
            rotate: Self::default_rotate(),
            dnssec: Self::default_dnssec(),
            ip_strategy: Self::default_ip_strategy(),
            cache_size: Self::default_cache_size(),
            use_hosts_file: Self::default_use_hosts_file(),
            num_concurrent_reqs: Self::default_num_concurrent_reqs(),
        }
    }
}

impl ResolverOptsWrapper {
    pub(crate) const fn default_timeout() -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }

    pub(crate) const fn default_attempts() -> usize {
        2
    }
    pub(crate) const fn default_rotate() -> bool {
        false
    }

    pub(crate) const fn default_dnssec() -> bool {
        false
    }

    pub(crate) fn default_ip_strategy() -> trust_dns_resolver::config::LookupIpStrategy {
        trust_dns_resolver::config::LookupIpStrategy::default()
    }

    pub(crate) const fn default_cache_size() -> usize {
        32
    }

    pub(crate) const fn default_use_hosts_file() -> bool {
        true
    }

    pub(crate) const fn default_num_concurrent_reqs() -> usize {
        2
    }
}

impl Default for FieldServerSMTPError {
    fn default() -> Self {
        Self {
            soft_count: 10,
            hard_count: 20,
            delay: std::time::Duration::from_millis(5000),
        }
    }
}

impl Default for FieldServerSMTPTimeoutClient {
    fn default() -> Self {
        Self {
            connect: std::time::Duration::from_secs(5 * 60),
            helo: std::time::Duration::from_secs(5 * 60),
            mail_from: std::time::Duration::from_secs(5 * 60),
            rcpt_to: std::time::Duration::from_secs(5 * 60),
            data: std::time::Duration::from_secs(5 * 60),
        }
    }
}

impl Default for FieldApp {
    fn default() -> Self {
        Self {
            dirpath: Self::default_dirpath(),
            vsl: FieldAppVSL::default(),
            logs: FieldAppLogs::default(),
        }
    }
}

impl FieldApp {
    pub(crate) fn default_dirpath() -> std::path::PathBuf {
        "/var/spool/vsmtp/app".into()
    }
}

impl Default for FieldAppLogs {
    fn default() -> Self {
        Self {
            filename: Self::default_filename(),
        }
    }
}

impl FieldAppLogs {
    pub(crate) fn default_filename() -> std::path::PathBuf {
        "/var/log/vsmtp/app.log".into()
    }
}
