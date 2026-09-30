use cad_platform::CancellationToken;
#[test]
fn clones_share_cancellation_without_claiming_computation_stopped() {
    let token = CancellationToken::default();
    let worker = token.clone();
    assert!(!worker.is_cancelled());
    token.cancel();
    assert!(worker.is_cancelled());
}
