#![allow(dead_code)]

use std::ptr;

use crate::ffi::{self, sqlite3, sqlite3_backup, sqlite3_session, sqlite3_stmt};

pub struct Connection {
    ptr: *mut sqlite3,
}

impl Connection {
    pub fn new(ptr: *mut sqlite3) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut sqlite3 {
        self.ptr
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    pub fn take(&mut self) -> *mut sqlite3 {
        let p = self.ptr;
        self.ptr = ptr::null_mut();
        p
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::sqlite3_close_v2(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

pub struct Statement {
    ptr: *mut sqlite3_stmt,
}

impl Statement {
    pub fn new(ptr: *mut sqlite3_stmt) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut sqlite3_stmt {
        self.ptr
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    pub fn take(&mut self) -> *mut sqlite3_stmt {
        let p = self.ptr;
        self.ptr = ptr::null_mut();
        p
    }
}

impl Drop for Statement {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::sqlite3_finalize(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

pub struct Session {
    ptr: *mut sqlite3_session,
}

impl Session {
    pub fn new(ptr: *mut sqlite3_session) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut sqlite3_session {
        self.ptr
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }

    pub fn take(&mut self) -> *mut sqlite3_session {
        let p = self.ptr;
        self.ptr = ptr::null_mut();
        p
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::sqlite3session_delete(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

pub struct Backup {
    ptr: *mut sqlite3_backup,
}

impl Backup {
    pub fn new(ptr: *mut sqlite3_backup) -> Self {
        Self { ptr }
    }

    pub fn as_ptr(&self) -> *mut sqlite3_backup {
        self.ptr
    }

    pub fn is_null(&self) -> bool {
        self.ptr.is_null()
    }
}

impl Drop for Backup {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                ffi::sqlite3_backup_finish(self.ptr);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

pub struct SqliteFree<T>(*mut T);

impl<T> SqliteFree<T> {
    pub fn new(ptr: *mut T) -> Self {
        Self(ptr)
    }

    pub fn as_ptr(&self) -> *mut T {
        self.0
    }

    pub fn take(self) -> *mut T {
        let p = self.0;
        std::mem::forget(self);
        p
    }
}

impl<T> Drop for SqliteFree<T> {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                ffi::sqlite3_free(self.0.cast());
            }
            self.0 = ptr::null_mut();
        }
    }
}
