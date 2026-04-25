use std::{fmt::Debug, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, bail};
use rustls::{
    CertificateError, ClientConfig, DistinguishedName, Error as TlsError, ServerConfig,
    SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    version::TLS13,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::{
    PROTOCOL_VERSION,
    frame::{Frame, read_frame, write_frame},
    secrets::{Identity, PeerIdentity},
};

const SERVER_NAME: &str = "rclippy.local";
const ALPN: &[u8] = b"rclippy-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionSide {
    Incoming,
    Outgoing,
}

pub type ClientTlsStream = tokio_rustls::client::TlsStream<TcpStream>;
pub type ServerTlsStream = tokio_rustls::server::TlsStream<TcpStream>;

pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

pub fn client_config(identity: &Identity, peer: &PeerIdentity) -> Result<ClientConfig> {
    install_crypto_provider();
    let supported = rustls::crypto::ring::default_provider().signature_verification_algorithms;
    let mut config = ClientConfig::builder_with_protocol_versions(&[&TLS13])
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedServerVerifier::new(
            peer.cert_der.clone(),
            supported,
        )))
        .with_client_auth_cert(vec![identity.cert()], identity.private_key())
        .context("build client TLS config")?;
    config.alpn_protocols.push(ALPN.to_vec());
    Ok(config)
}

pub fn server_config(identity: &Identity, peer: &PeerIdentity) -> Result<ServerConfig> {
    install_crypto_provider();
    let supported = rustls::crypto::ring::default_provider().signature_verification_algorithms;
    let verifier = Arc::new(PinnedClientVerifier::new(peer.cert_der.clone(), supported));

    let mut config = ServerConfig::builder_with_protocol_versions(&[&TLS13])
        .with_client_cert_verifier(verifier)
        .with_single_cert(vec![identity.cert()], identity.private_key())
        .context("build server TLS config")?;
    config.alpn_protocols.push(ALPN.to_vec());
    Ok(config)
}

pub async fn connect_tls(
    addr: SocketAddr,
    identity: &Identity,
    peer: &PeerIdentity,
) -> Result<ClientTlsStream> {
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("connect peer {addr}"))?;
    let connector = TlsConnector::from(Arc::new(client_config(identity, peer)?));
    let server_name = ServerName::try_from(SERVER_NAME)
        .context("parse TLS server name")?
        .to_owned();
    connector
        .connect(server_name, stream)
        .await
        .context("TLS client handshake")
}

pub async fn accept_tls(
    listener: &TcpListener,
    identity: &Identity,
    peer: &PeerIdentity,
) -> Result<ServerTlsStream> {
    let (stream, _) = listener.accept().await.context("accept peer TCP")?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config(identity, peer)?));
    acceptor
        .accept(stream)
        .await
        .context("TLS server handshake")
}

pub async fn client_hello(stream: &mut ClientTlsStream, identity: &Identity) -> Result<String> {
    write_frame(stream, &Frame::hello(&identity.device_id)).await?;
    read_peer_hello(stream).await
}

pub async fn server_hello(stream: &mut ServerTlsStream, identity: &Identity) -> Result<String> {
    let peer_id = read_peer_hello(stream).await?;
    write_frame(stream, &Frame::hello(&identity.device_id)).await?;
    Ok(peer_id)
}

pub async fn read_peer_hello<S>(stream: &mut S) -> Result<String>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let frame = read_frame(stream).await?;
    match frame {
        Frame::Hello {
            protocol_version,
            device_id,
        } => {
            if protocol_version != PROTOCOL_VERSION {
                bail!("unsupported peer protocol version {protocol_version}");
            }
            Ok(device_id)
        }
        _ => bail!("expected peer hello"),
    }
}

pub fn should_keep_connection(
    local_device_id: &str,
    peer_device_id: &str,
    side: ConnectionSide,
) -> bool {
    match local_device_id.cmp(peer_device_id) {
        std::cmp::Ordering::Less => side == ConnectionSide::Outgoing,
        std::cmp::Ordering::Greater => side == ConnectionSide::Incoming,
        std::cmp::Ordering::Equal => false,
    }
}

pub fn verify_expected_peer(actual: &str, expected: &PeerIdentity) -> Result<()> {
    if actual != expected.device_id {
        bail!("peer device id mismatch");
    }
    Ok(())
}

#[derive(Debug)]
struct PinnedServerVerifier {
    cert_der: Vec<u8>,
    supported: WebPkiSupportedAlgorithms,
}

impl PinnedServerVerifier {
    fn new(cert_der: Vec<u8>, supported: WebPkiSupportedAlgorithms) -> Self {
        Self {
            cert_der,
            supported,
        }
    }
}

impl ServerCertVerifier for PinnedServerVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if end_entity.as_ref() == self.cert_der.as_slice() {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

#[derive(Debug)]
struct PinnedClientVerifier {
    cert_der: Vec<u8>,
    supported: WebPkiSupportedAlgorithms,
    root_hints: Vec<DistinguishedName>,
}

impl PinnedClientVerifier {
    fn new(cert_der: Vec<u8>, supported: WebPkiSupportedAlgorithms) -> Self {
        Self {
            cert_der,
            supported,
            root_hints: Vec::new(),
        }
    }
}

impl ClientCertVerifier for PinnedClientVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.root_hints
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        if end_entity.as_ref() == self.cert_der.as_slice() {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(TlsError::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::generate_identity;

    #[test]
    fn deterministic_connection_winner() {
        assert!(should_keep_connection(
            "aaa",
            "bbb",
            ConnectionSide::Outgoing
        ));
        assert!(!should_keep_connection(
            "aaa",
            "bbb",
            ConnectionSide::Incoming
        ));
        assert!(should_keep_connection(
            "bbb",
            "aaa",
            ConnectionSide::Incoming
        ));
        assert!(!should_keep_connection(
            "bbb",
            "aaa",
            ConnectionSide::Outgoing
        ));
    }

    #[test]
    fn builds_tls_configs_with_pinned_cert() {
        let a = generate_identity().unwrap();
        let b = generate_identity().unwrap();
        let peer_b = PeerIdentity {
            device_id: b.device_id.clone(),
            device_name: "b".to_owned(),
            cert_der: b.cert_der.clone(),
        };
        let peer_a = PeerIdentity {
            device_id: a.device_id.clone(),
            device_name: "a".to_owned(),
            cert_der: a.cert_der.clone(),
        };

        client_config(&a, &peer_b).unwrap();
        server_config(&b, &peer_a).unwrap();
    }
}
