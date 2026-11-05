use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};
use zeron_update::{UPSTREAM_RELEASES, Updater, fetch_latest};

async fn local_edge() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    (listener, url)
}

async fn accepted(listener: &TcpListener) -> bool {
    tokio::time::timeout(Duration::from_millis(300), listener.accept())
        .await
        .is_ok()
}

#[test]
fn upstream_releases_are_off() {
    assert!(!UPSTREAM_RELEASES);
}

#[tokio::test]
async fn the_probe_sees_a_connection() {
    let (listener, url) = local_edge().await;
    let _stream = TcpStream::connect(url.trim_start_matches("http://"))
        .await
        .unwrap();
    assert!(accepted(&listener).await);
}

#[tokio::test]
async fn fetch_latest_fails_without_connecting() {
    let (listener, url) = local_edge().await;
    assert!(fetch_latest(&url).await.is_err());
    assert!(!accepted(&listener).await);
}

#[tokio::test]
async fn user_check_fails_without_connecting() {
    let (listener, url) = local_edge().await;
    let updater = Updater::spawn_desktop(url);
    assert!(updater.check().await.is_err());
    assert!(!accepted(&listener).await);
    updater.shutdown().await;
}

#[tokio::test]
async fn background_checker_never_connects() {
    let (listener, url) = local_edge().await;
    let updater = Updater::spawn(url, None);
    updater.check_now();
    assert!(!accepted(&listener).await);
    let status = updater.watch().borrow().clone();
    assert_eq!(status.checked_at, None);
    assert_eq!(status.error, None);
    updater.shutdown().await;
}

#[tokio::test]
async fn release_download_fails_without_connecting() {
    let (listener, url) = local_edge().await;
    let dest = std::env::temp_dir().join(format!("nova-lock-{}", std::process::id()));
    let manifest = zeron_update::Manifest::default();
    assert!(
        zeron_update::download_release_file(&url, &manifest, "zeron.tar.gz", &dest)
            .await
            .is_err()
    );
    assert!(!accepted(&listener).await);
    assert!(!dest.exists());
}
