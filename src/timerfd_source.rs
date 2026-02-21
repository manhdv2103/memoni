use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};
use std::os::fd::{AsFd, OwnedFd};

pub struct TimerfdSource {
    fd: OwnedFd,
}

impl TimerfdSource {
    pub fn new() -> Result<Self, rustix::io::Errno> {
        let fd = timerfd_create(
            TimerfdClockId::Monotonic,
            TimerfdFlags::NONBLOCK | TimerfdFlags::CLOEXEC,
        )?;
        Ok(Self { fd })
    }

    pub fn set_timer(&self, ms: u64) -> Result<(), rustix::io::Errno> {
        let secs = (ms / 1000) as i64;
        let nanos = ((ms % 1000) * 1_000_000) as i64;
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: secs,
                tv_nsec: nanos,
            },
        };
        timerfd_settime(&self.fd, TimerfdTimerFlags::empty(), &spec)?;
        Ok(())
    }

    pub fn disarm(&self) -> Result<(), rustix::io::Errno> {
        let spec = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
        };
        timerfd_settime(&self.fd, TimerfdTimerFlags::empty(), &spec)?;
        Ok(())
    }

    pub fn clear_event(&self) -> std::io::Result<u64> {
        let mut buf = [0u8; 8];
        let n = rustix::io::read(&self.fd, &mut buf)?;
        if n == 8 {
            Ok(u64::from_ne_bytes(buf))
        } else {
            Ok(0)
        }
    }
}

impl AsFd for TimerfdSource {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.fd.as_fd()
    }
}
