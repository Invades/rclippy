use rclippy::{
    frame::{Frame, read_frame, write_frame},
    secrets::{PeerIdentity, generate_identity},
    transport::{accept_tls, client_hello, connect_tls, server_hello, verify_expected_peer},
};
use tokio::net::TcpListener;

#[tokio::test]
async fn paired_transport_moves_clipboard_frame() {
    rclippy::transport::install_crypto_provider();

    let a = generate_identity().unwrap();
    let b = generate_identity().unwrap();
    let peer_a = PeerIdentity {
        device_id: a.device_id.clone(),
        device_name: "a".to_owned(),
        cert_der: a.cert_der.clone(),
    };
    let peer_b = PeerIdentity {
        device_id: b.device_id.clone(),
        device_name: "b".to_owned(),
        cert_der: b.cert_der.clone(),
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn({
        let b = b.clone();
        let peer_a = peer_a.clone();
        async move {
            let mut stream = accept_tls(&listener, &b, &peer_a).await.unwrap();
            let peer_id = server_hello(&mut stream, &b).await.unwrap();
            verify_expected_peer(&peer_id, &peer_a).unwrap();
            read_frame(&mut stream).await.unwrap()
        }
    });

    let mut client = connect_tls(addr, &a, &peer_b).await.unwrap();
    let peer_id = client_hello(&mut client, &a).await.unwrap();
    verify_expected_peer(&peer_id, &peer_b).unwrap();
    let frame = Frame::clipboard_text(1, "hello over tls".to_owned());
    write_frame(&mut client, &frame).await.unwrap();

    assert_eq!(server.await.unwrap(), frame);
}
