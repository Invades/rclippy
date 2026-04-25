use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::{Rng, RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout},
};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::secrets::{Identity, PeerIdentity};

type HmacSha256 = Hmac<Sha256>;

const PAIRING_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PAIR_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PairHello {
    device_id: String,
    #[serde(default)]
    device_name: String,
    cert_der: String,
    eph_pub: String,
    nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum PairFrame {
    Hello(PairHello),
    Proof { proof: String },
}

pub fn generate_pairing_code() -> String {
    let mut rng = OsRng;
    format!(
        "{:03}-{:03}",
        rng.gen_range(0..1000),
        rng.gen_range(0..1000)
    )
}

pub async fn host_pairing_once(
    listen_addr: SocketAddr,
    code: &str,
    identity: &Identity,
) -> Result<PeerIdentity> {
    let listener = TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("bind pairing listener {listen_addr}"))?;
    host_pairing_with_listener(listener, code, identity).await
}

async fn host_pairing_with_listener(
    listener: TcpListener,
    code: &str,
    identity: &Identity,
) -> Result<PeerIdentity> {
    let deadline = Instant::now() + PAIRING_TIMEOUT;

    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("pairing timed out")?;
        let (stream, _) = timeout(remaining, listener.accept())
            .await
            .context("pairing timed out")??;

        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("pairing timed out")?;
        match timeout(remaining, handle_host_pairing(stream, code, identity)).await {
            Ok(Ok(peer)) => return Ok(peer),
            Ok(Err(err)) if is_ignorable_pairing_probe(&err) => continue,
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err(anyhow!("pairing timed out")),
        }
    }
}

pub async fn join_pairing(
    peer_addr: SocketAddr,
    code: &str,
    identity: &Identity,
) -> Result<PeerIdentity> {
    let stream = timeout(PAIRING_TIMEOUT, TcpStream::connect(peer_addr))
        .await
        .context("pairing connection timed out")?
        .with_context(|| format!("connect pairing peer {peer_addr}"))?;
    handle_client_pairing(stream, code, identity).await
}

async fn handle_host_pairing(
    mut stream: TcpStream,
    code: &str,
    identity: &Identity,
) -> Result<PeerIdentity> {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    let mut nonce = [0_u8; 32];
    OsRng.fill_bytes(&mut nonce);

    let PairFrame::Hello(client_hello) = read_pair_frame(&mut stream).await? else {
        bail!("expected client pairing hello");
    };
    let host_hello = build_hello(identity, public.as_bytes(), &nonce);
    write_pair_frame(&mut stream, &PairFrame::Hello(host_hello.clone())).await?;

    let key = derive_pair_key(code, &secret, &client_hello, &host_hello)?;
    let transcript = transcript_bytes(&client_hello, &host_hello)?;
    let expected_client = proof(&key, b"client", &transcript);

    let PairFrame::Proof {
        proof: client_proof,
    } = read_pair_frame(&mut stream).await?
    else {
        bail!("expected client pairing proof");
    };
    if decode_b64(&client_proof)? != expected_client {
        bail!("pairing code proof failed");
    }

    let host_proof = STANDARD_NO_PAD.encode(proof(&key, b"host", &transcript));
    write_pair_frame(&mut stream, &PairFrame::Proof { proof: host_proof }).await?;

    peer_from_hello(client_hello)
}

async fn handle_client_pairing(
    mut stream: TcpStream,
    code: &str,
    identity: &Identity,
) -> Result<PeerIdentity> {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = PublicKey::from(&secret);
    let mut nonce = [0_u8; 32];
    OsRng.fill_bytes(&mut nonce);

    let client_hello = build_hello(identity, public.as_bytes(), &nonce);
    write_pair_frame(&mut stream, &PairFrame::Hello(client_hello.clone())).await?;

    let PairFrame::Hello(host_hello) = read_pair_frame(&mut stream).await? else {
        bail!("expected host pairing hello");
    };

    let key = derive_pair_key(code, &secret, &client_hello, &host_hello)?;
    let transcript = transcript_bytes(&client_hello, &host_hello)?;
    let client_proof = STANDARD_NO_PAD.encode(proof(&key, b"client", &transcript));
    write_pair_frame(
        &mut stream,
        &PairFrame::Proof {
            proof: client_proof,
        },
    )
    .await?;

    let PairFrame::Proof { proof: host_proof } = read_pair_frame(&mut stream).await? else {
        bail!("expected host pairing proof");
    };
    let expected_host = proof(&key, b"host", &transcript);
    if decode_b64(&host_proof)? != expected_host {
        bail!("pairing code proof failed");
    }

    peer_from_hello(host_hello)
}

fn build_hello(identity: &Identity, eph_pub: &[u8], nonce: &[u8]) -> PairHello {
    PairHello {
        device_id: identity.device_id.clone(),
        device_name: local_device_name(),
        cert_der: STANDARD_NO_PAD.encode(&identity.cert_der),
        eph_pub: STANDARD_NO_PAD.encode(eph_pub),
        nonce: STANDARD_NO_PAD.encode(nonce),
    }
}

fn peer_from_hello(hello: PairHello) -> Result<PeerIdentity> {
    Ok(PeerIdentity {
        device_id: hello.device_id,
        device_name: hello.device_name,
        cert_der: decode_b64(&hello.cert_der)?,
    })
}

fn local_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Unknown device".to_owned())
}

fn derive_pair_key(
    code: &str,
    own_secret: &StaticSecret,
    client_hello: &PairHello,
    host_hello: &PairHello,
) -> Result<[u8; 32]> {
    let peer_pub = if own_public_matches(own_secret, client_hello)? {
        decode_public_key(&host_hello.eph_pub)?
    } else {
        decode_public_key(&client_hello.eph_pub)?
    };
    let shared = own_secret.diffie_hellman(&peer_pub);
    let transcript = transcript_bytes(client_hello, host_hello)?;
    derive_pair_key_bytes(code, shared.as_bytes(), &transcript)
}

fn own_public_matches(secret: &StaticSecret, hello: &PairHello) -> Result<bool> {
    let own_public = PublicKey::from(secret);
    Ok(own_public.as_bytes().as_slice() == decode_b64(&hello.eph_pub)?.as_slice())
}

fn derive_pair_key_bytes(code: &str, shared: &[u8], transcript: &[u8]) -> Result<[u8; 32]> {
    let hkdf = Hkdf::<Sha256>::new(Some(code.as_bytes()), shared);
    let mut key = [0_u8; 32];
    hkdf.expand(transcript, &mut key)
        .map_err(|_| anyhow!("derive pairing key"))?;
    Ok(key)
}

fn proof(key: &[u8], role: &[u8], transcript: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(b"rclippy pairing proof v1");
    mac.update(role);
    mac.update(transcript);
    mac.finalize().into_bytes().to_vec()
}

fn transcript_bytes(client_hello: &PairHello, host_hello: &PairHello) -> Result<Vec<u8>> {
    serde_json::to_vec(&(client_hello, host_hello)).context("serialize pairing transcript")
}

fn decode_public_key(value: &str) -> Result<PublicKey> {
    let bytes = decode_b64(value)?;
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("invalid pairing public key length"))?;
    Ok(PublicKey::from(array))
}

fn decode_b64(value: &str) -> Result<Vec<u8>> {
    STANDARD_NO_PAD
        .decode(value.as_bytes())
        .context("decode base64")
}

fn is_ignorable_pairing_probe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        let message = cause.to_string();
        message == "pair frame too large"
    })
}

async fn write_pair_frame<W>(writer: &mut W, frame: &PairFrame) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(frame).map_err(|err| std::io::Error::other(err.to_string()))?;
    if bytes.len() > MAX_PAIR_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pair frame too large",
        ));
    }
    writer.write_u32(bytes.len() as u32).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await
}

async fn read_pair_frame<R>(reader: &mut R) -> std::io::Result<PairFrame>
where
    R: AsyncRead + Unpin,
{
    let len = reader.read_u32().await? as usize;
    if len > MAX_PAIR_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pair frame too large",
        ));
    }
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|err| std::io::Error::other(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::generate_identity;

    #[test]
    fn different_codes_make_different_keys() {
        let identity = generate_identity().unwrap();
        let secret_a = StaticSecret::random_from_rng(OsRng);
        let public_a = PublicKey::from(&secret_a);
        let secret_b = StaticSecret::random_from_rng(OsRng);
        let public_b = PublicKey::from(&secret_b);
        let hello_a = build_hello(&identity, public_a.as_bytes(), &[1; 32]);
        let hello_b = build_hello(&identity, public_b.as_bytes(), &[2; 32]);
        let shared = secret_a.diffie_hellman(&public_b);
        let transcript = transcript_bytes(&hello_a, &hello_b).unwrap();

        let key_a = derive_pair_key_bytes("111-111", shared.as_bytes(), &transcript).unwrap();
        let key_b = derive_pair_key_bytes("222-222", shared.as_bytes(), &transcript).unwrap();

        assert_ne!(key_a, key_b);
    }

    #[tokio::test]
    async fn pairing_succeeds_with_matching_code() {
        let host = generate_identity().unwrap();
        let client = generate_identity().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let code = "123-456".to_owned();

        let host_for_task = host.clone();
        let host_task = tokio::spawn({
            let code = code.clone();
            async move {
                let (stream, _) = listener.accept().await.unwrap();
                handle_host_pairing(stream, &code, &host_for_task)
                    .await
                    .unwrap()
            }
        });
        let client_peer = join_pairing(addr, &code, &client).await.unwrap();
        let host_peer = host_task.await.unwrap();

        assert_eq!(host_peer.device_id, client.device_id);
        assert_eq!(client_peer.device_id, host.device_id);
    }

    #[tokio::test]
    async fn host_pairing_ignores_tls_probe_before_real_pairing() {
        let host = generate_identity().unwrap();
        let client = generate_identity().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let code = "123-456".to_owned();

        let host_for_task = host.clone();
        let host_task = tokio::spawn({
            let code = code.clone();
            async move {
                host_pairing_with_listener(listener, &code, &host_for_task)
                    .await
                    .unwrap()
            }
        });

        let mut probe = TcpStream::connect(addr).await.unwrap();
        probe.write_all(&[0xff, 0xff, 0xff, 0xff]).await.unwrap();
        drop(probe);

        let client_peer = join_pairing(addr, &code, &client).await.unwrap();
        let host_peer = host_task.await.unwrap();

        assert_eq!(host_peer.device_id, client.device_id);
        assert_eq!(client_peer.device_id, host.device_id);
    }

    #[tokio::test]
    async fn pairing_fails_with_wrong_code() {
        let host = generate_identity().unwrap();
        let client = generate_identity().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let host_task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_host_pairing(stream, "123-456", &host).await
        });
        let client_result = join_pairing(addr, "999-999", &client).await;
        let host_result = host_task.await.unwrap();

        assert!(client_result.is_err() || host_result.is_err());
    }
}
