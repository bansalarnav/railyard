use crate::daemon;
use std::io;

pub(crate) fn run() -> io::Result<()> {
    daemon::down()
}
