//! SMTP delivery for caregiver bundle emails.
//!
//! Three providers in v1:
//! - Gmail (smtp.gmail.com:587 STARTTLS, app-password auth)
//! - iCloud (smtp.mail.me.com:587 STARTTLS, app-password auth)
//! - Generic (user supplies host/port, STARTTLS auto-detected)
//!
//! Credentials live in the OS keychain (`keyring` crate). They
//! are NOT round-tripped through serde — the only IPC entry
//! point is `smtp_save_credential`, which writes directly to
//! Keychain and never returns the secret.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use super::consent::SmtpProvider;

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_starttls: bool,
}

impl SmtpConfig {
    /// Provider presets. The user supplies the username (their
    /// email) and password (an app-password); host/port are
    /// fixed.
    pub fn from_provider(provider: &SmtpProvider, username: String, password: String) -> Self {
        match provider {
            SmtpProvider::GmailAppPassword => Self {
                host: "smtp.gmail.com".into(),
                port: 587,
                username,
                password,
                use_starttls: true,
            },
            SmtpProvider::IcloudAppPassword => Self {
                host: "smtp.mail.me.com".into(),
                port: 587,
                username,
                password,
                use_starttls: true,
            },
            SmtpProvider::Generic => Self {
                host: "localhost".into(),
                port: 587,
                username,
                password,
                use_starttls: true,
            },
        }
    }

    pub fn generic(
        host: String,
        port: u16,
        username: String,
        password: String,
        use_starttls: bool,
    ) -> Self {
        Self {
            host,
            port,
            username,
            password,
            use_starttls,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SmtpError {
    #[error("invalid email address: {0}")]
    InvalidEmail(String),
    #[error("SMTP transport: {0}")]
    Transport(String),
    #[error("SMTP send: {0}")]
    Send(String),
}

/// Send a plain-text email. Returns Ok on a successful 2xx
/// response from the server.
pub async fn send_email(
    config: &SmtpConfig,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), SmtpError> {
    let from_mbox = from
        .parse::<lettre::Address>()
        .map_err(|e| SmtpError::InvalidEmail(format!("from {from}: {e}")))?;
    let to_mbox = to
        .parse::<lettre::Address>()
        .map_err(|e| SmtpError::InvalidEmail(format!("to {to}: {e}")))?;

    let message = Message::builder()
        .from(from_mbox.into())
        .to(to_mbox.into())
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|e| SmtpError::Send(format!("build message: {e}")))?;

    let creds = Credentials::new(config.username.clone(), config.password.clone());

    let mailer = if config.use_starttls {
        let tls = TlsParameters::new(config.host.clone())
            .map_err(|e| SmtpError::Transport(format!("TLS params for {}: {e}", config.host)))?;
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .tls(Tls::Required(tls))
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
            .port(config.port)
            .credentials(creds)
            .build()
    };

    mailer
        .send(message)
        .await
        .map_err(|e| SmtpError::Send(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_preset_uses_smtp_gmail_587_starttls() {
        let c = SmtpConfig::from_provider(
            &SmtpProvider::GmailAppPassword,
            "user@example.com".into(),
            "secret".into(),
        );
        assert_eq!(c.host, "smtp.gmail.com");
        assert_eq!(c.port, 587);
        assert!(c.use_starttls);
    }

    #[test]
    fn icloud_preset_uses_smtp_mail_me_587_starttls() {
        let c = SmtpConfig::from_provider(
            &SmtpProvider::IcloudAppPassword,
            "user@icloud.com".into(),
            "secret".into(),
        );
        assert_eq!(c.host, "smtp.mail.me.com");
        assert_eq!(c.port, 587);
        assert!(c.use_starttls);
    }

    #[test]
    fn invalid_email_addresses_rejected() {
        // Cheap sync test of the address parser. Don't run real
        // SMTP — that's covered by manual integration testing
        // on the dev's machine.
        let e1 = "user@example.com".parse::<lettre::Address>();
        assert!(e1.is_ok());
        let e2 = "not an email".parse::<lettre::Address>();
        assert!(e2.is_err());
    }
}
