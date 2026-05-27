use crate::daemon;

pub(crate) fn run() {
    daemon::restart();
}
