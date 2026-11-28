//! Whether the clipboard changed since this app last wrote to it: Windows counts every change (any program's) in
//! one number. The window could only find out by reading the clipboard, which WebView2 asks the user about first.

/// The clipboard's change count (None where there is none to ask).
pub fn sequence() -> Option<u32> {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetClipboardSequenceNumber() -> u32;
        }
        // SAFETY: takes nothing and only returns a number; it never fails (0 means no access to the
        // clipboard's window station, which is told apart below).
        #[allow(unsafe_code)]
        let n = unsafe { GetClipboardSequenceNumber() };
        if n == 0 {
            None
        } else {
            Some(n)
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_count_is_there_and_steady_while_nothing_writes() {
        let a = super::sequence();
        let b = super::sequence();
        if cfg!(windows) {
            assert!(a.is_some());
            // Another program may write in between; two reads in a row almost never differ.
            assert!(b >= a);
        } else {
            assert_eq!(a, None);
        }
    }
}
