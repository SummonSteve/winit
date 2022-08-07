use std::{
    ffi::{c_void, OsString},
    mem::zeroed,
    os::windows::prelude::OsStringExt,
    ptr::null_mut,
};

use windows_sys::Win32::{
    Foundation::POINT,
    Globalization::HIMC,
    UI::{
        Input::Ime::{
            ImmGetCandidateListW, ImmAssociateContextEx, ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext,
            ImmSetCandidateWindow, ATTR_TARGET_CONVERTED, ATTR_TARGET_NOTCONVERTED, CANDIDATEFORM,
            CFS_EXCLUDE, GCS_COMPATTR, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR, IACE_CHILDREN,
            IACE_DEFAULT, CANDIDATELIST
        },
        WindowsAndMessaging::{GetSystemMetrics, SM_IMMENABLED},
    },
};

use crate::{dpi::Position, platform::windows::HWND};

pub struct ImeContext {
    hwnd: HWND,
    himc: HIMC,
}

impl ImeContext {
    pub unsafe fn current(hwnd: HWND) -> Self {
        let himc = ImmGetContext(hwnd);
        ImeContext { hwnd, himc }
    }

    pub unsafe fn get_composing_text_and_cursor(
        &self,
    ) -> Option<(String, Option<usize>, Option<usize>)> {
        let text = self.get_composition_string(GCS_COMPSTR)?;
        let attrs = self.get_composition_data(GCS_COMPATTR).unwrap_or_default();

        let mut first = None;
        let mut last = None;
        let mut boundary_before_char = 0;

        for (attr, chr) in attrs.into_iter().zip(text.chars()) {
            let char_is_targetted =
                attr as u32 == ATTR_TARGET_CONVERTED || attr as u32 == ATTR_TARGET_NOTCONVERTED;

            if first.is_none() && char_is_targetted {
                first = Some(boundary_before_char);
            } else if first.is_some() && last.is_none() && !char_is_targetted {
                last = Some(boundary_before_char);
            }

            boundary_before_char += chr.len_utf8();
        }

        if first.is_some() && last.is_none() {
            last = Some(text.len());
        } else if first.is_none() {
            // IME haven't split words and select any clause yet, so trying to retrieve normal cursor.
            let cursor = self.get_composition_cursor(&text);
            first = cursor;
            last = cursor;
        }

        Some((text, first, last))
    }

    pub unsafe fn get_composed_text(&self) -> Option<String> {
        self.get_composition_string(GCS_RESULTSTR)
    }

    pub unsafe fn get_candidate_list(&self) -> Option<Vec<String>> {
        let size = ImmGetCandidateListW(self.himc, 0, std::ptr::null_mut(), 0) as usize;
        if size == 0 {
            return None;
        }
        let mut buf: Vec<u8> = Vec::with_capacity(size);
        let ret = ImmGetCandidateListW(self.himc, 0, buf.as_mut_ptr() as *mut _, size as u32);
        if ret == 0 {
            return None;
        }
        buf.set_len(size);
        let obj = &*(buf.as_ptr() as *const CANDIDATELIST);
        let mut list: Vec<String> = Vec::with_capacity(obj.dwCount as usize);
        for i in 0..(obj.dwCount as usize) {
            let offset =
                std::slice::from_raw_parts(&obj.dwOffset as *const u32, obj.dwCount as usize);
            let p = buf.as_ptr().offset(offset[i] as isize) as *const u16;
            let len = (0..isize::MAX).position(|i| *p.offset(i) == 0).unwrap();
            let slice = std::slice::from_raw_parts(p, len);
            list.push(String::from_utf16_lossy(slice));
        }
        Some(list)
    }

    unsafe fn get_composition_cursor(&self, text: &str) -> Option<usize> {
        let cursor = ImmGetCompositionStringW(self.himc, GCS_CURSORPOS, null_mut(), 0);
        (cursor >= 0).then(|| text.chars().take(cursor as _).map(|c| c.len_utf8()).sum())
    }

    unsafe fn get_composition_string(&self, gcs_mode: u32) -> Option<String> {
        let data = self.get_composition_data(gcs_mode)?;
        let (prefix, shorts, suffix) = data.align_to::<u16>();
        if prefix.is_empty() && suffix.is_empty() {
            OsString::from_wide(shorts).into_string().ok()
        } else {
            None
        }
    }

    unsafe fn get_composition_data(&self, gcs_mode: u32) -> Option<Vec<u8>> {
        let size = match ImmGetCompositionStringW(self.himc, gcs_mode, null_mut(), 0) {
            0 => return Some(Vec::new()),
            size if size < 0 => return None,
            size => size,
        };

        let mut buf = Vec::<u8>::with_capacity(size as _);
        let size = ImmGetCompositionStringW(
            self.himc,
            gcs_mode,
            buf.as_mut_ptr() as *mut c_void,
            size as _,
        );

        if size < 0 {
            None
        } else {
            buf.set_len(size as _);
            Some(buf)
        }
    }

    pub unsafe fn set_ime_position(&self, spot: Position, scale_factor: f64) {
        if !ImeContext::system_has_ime() {
            return;
        }

        let (x, y) = spot.to_physical::<i32>(scale_factor).into();
        let candidate_form = CANDIDATEFORM {
            dwIndex: 0,
            dwStyle: CFS_EXCLUDE,
            ptCurrentPos: POINT { x, y },
            rcArea: zeroed(),
        };

        ImmSetCandidateWindow(self.himc, &candidate_form);
    }

    pub unsafe fn set_ime_allowed(hwnd: HWND, allowed: bool) {
        if !ImeContext::system_has_ime() {
            return;
        }

        if allowed {
            ImmAssociateContextEx(hwnd, 0, IACE_DEFAULT);
        } else {
            ImmAssociateContextEx(hwnd, 0, IACE_CHILDREN);
        }
    }

    unsafe fn system_has_ime() -> bool {
        GetSystemMetrics(SM_IMMENABLED) != 0
    }
}

impl Drop for ImeContext {
    fn drop(&mut self) {
        unsafe { ImmReleaseContext(self.hwnd, self.himc) };
    }
}
