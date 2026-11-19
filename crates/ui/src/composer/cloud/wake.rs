//! Foreground admission and held queueing have different wake intentions.
//! A foreground message authorizes resume; saving held input never does.
pub(super) fn intent(queue: bool, _sleeping: bool) -> &'static str {
    if queue {
        "preserve-sleep"
    } else {
        "explicit-resume"
    }
}
