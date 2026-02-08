
#[test]
fn test_engine_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<wilysearch::engine::Engine>();
}
