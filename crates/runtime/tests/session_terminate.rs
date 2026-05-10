//! Runtime session termination orchestration tests.

use std::time::Duration;

use firkin_admission::{CapacityLedger, ResourceBudget};
use firkin_runtime::types::Size;
use firkin_runtime::{
    ActiveSessionReservation, RuntimeSessionTerminate, SessionTerminationError,
    SessionTerminationRequest, SessionTerminator,
};

#[derive(Default)]
struct RecordingTerminator {
    terminated_sessions: Vec<String>,
    fail: bool,
}

impl SessionTerminator for RecordingTerminator {
    type Error = &'static str;

    fn terminate_session(
        &mut self,
        request: &SessionTerminationRequest<'_>,
    ) -> Result<(), Self::Error> {
        self.terminated_sessions
            .push(request.session_id().to_owned());
        if self.fail {
            Err("terminate failed")
        } else {
            Ok(())
        }
    }
}

#[test]
fn session_terminate_releases_active_capacity_and_records_latency() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    ledger.reserve_active(budget).expect("reserve active");
    let mut reservation = ActiveSessionReservation::new(budget);
    let mut terminator = RecordingTerminator::default();

    let report = RuntimeSessionTerminate::new(&mut ledger, &mut reservation, "session-1")
        .execute_with_elapsed(&mut terminator, Duration::from_millis(41))
        .expect("terminate");

    assert_eq!(terminator.terminated_sessions, vec!["session-1"]);
    assert_eq!(
        ledger.active(),
        ResourceBudget::new(0, Size::bytes(0), Size::bytes(0))
    );
    assert_eq!(report.released_budget(), Some(budget));
    assert_eq!(report.benchmark_samples()[0].metric(), "kill_delete");
    assert!((report.benchmark_samples()[0].value() - 41.0).abs() < f64::EPSILON);
}

#[test]
fn session_terminate_keeps_capacity_when_termination_fails() {
    let mut ledger = CapacityLedger::new(ResourceBudget::new(8, Size::gib(64), Size::gib(512)));
    let budget = ResourceBudget::new(2, Size::gib(8), Size::gib(64));
    ledger.reserve_active(budget).expect("reserve active");
    let mut reservation = ActiveSessionReservation::new(budget);
    let mut terminator = RecordingTerminator {
        fail: true,
        ..RecordingTerminator::default()
    };

    let error = RuntimeSessionTerminate::new(&mut ledger, &mut reservation, "session-1")
        .execute_with_elapsed(&mut terminator, Duration::from_millis(41))
        .expect_err("terminate fails");

    assert!(matches!(error, SessionTerminationError::Terminate { .. }));
    assert_eq!(ledger.active(), budget);
    assert_eq!(reservation.release_into(&mut ledger), Some(budget));
}
