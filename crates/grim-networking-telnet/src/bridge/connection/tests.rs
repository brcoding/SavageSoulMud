use super::*;
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test]
async fn quiet_writer_sends_no_ops_and_drains_before_closing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let (read, write) = tokio::io::split(server);
    drop(read);
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tx.send(b"room".to_vec()).await.unwrap();
    let writer = tokio::spawn(write_connection(
        write,
        rx,
        iac::WONT_ECHO.to_vec(),
        Duration::from_millis(20),
    ));
    let mut prefix = [0; 7];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut prefix))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&prefix[..3], &iac::WONT_ECHO);
    assert_eq!(&prefix[3..], b"room");
    for _ in 0..2 {
        let mut heartbeat = [0; 2];
        tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut heartbeat))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(heartbeat, iac::NO_OP);
        assert!(iac::Decoder::default().feed(&heartbeat).unwrap().is_empty());
    }
    tx.send(b"bye".to_vec()).await.unwrap();
    drop(tx);
    tokio::time::timeout(Duration::from_secs(2), writer)
        .await
        .unwrap()
        .unwrap();
    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    let frames = iac::Decoder::default().feed(&tail).unwrap();
    assert_eq!(
        frames,
        b"bye"
            .iter()
            .copied()
            .map(iac::Frame::Data)
            .collect::<Vec<_>>()
    );
}
