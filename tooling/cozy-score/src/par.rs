//! A tiny deterministic parallel map. Results come back in index order, and
//! each item is computed independently, so the output never depends on how
//! the work was scheduled.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// `(0..n).map(f)`, spread over the available cores.
pub fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |t| t.get())
        .min(n);
    if threads <= 1 {
        return (0..n).map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<T>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let v = f(i);
                slots.lock().expect("no panics while holding the lock")[i] = Some(v);
            });
        }
    });
    slots
        .into_inner()
        .expect("lock")
        .into_iter()
        .map(|v| v.expect("every index computed"))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps_order() {
        let v = super::par_map(100, |i| i * i);
        assert_eq!(v, (0..100).map(|i| i * i).collect::<Vec<_>>());
    }
}
