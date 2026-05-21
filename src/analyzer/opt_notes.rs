use std::cell::RefCell;

thread_local! {
    static OPT_NOTES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static OPT_VERBOSE: RefCell<bool> = const { RefCell::new(false) };
    static CHANGE_COUNT: RefCell<u32> = const { RefCell::new(0) };
}

pub fn set_verbose(v: bool) {
    OPT_VERBOSE.with(|vb| *vb.borrow_mut() = v);
}

pub fn note(msg: impl Into<String>) {
    CHANGE_COUNT.with(|c| *c.borrow_mut() += 1);
    OPT_VERBOSE.with(|vb| {
        if *vb.borrow() {
            let s = msg.into();
            OPT_NOTES.with(|n| n.borrow_mut().push(s));
        }
    });
}

pub fn drain_notes() -> Vec<String> {
    OPT_NOTES.with(|n| std::mem::take(&mut *n.borrow_mut()))
}

pub fn reset_changes() {
    CHANGE_COUNT.with(|c| *c.borrow_mut() = 0);
}

pub fn change_count() -> u32 {
    CHANGE_COUNT.with(|c| *c.borrow())
}
