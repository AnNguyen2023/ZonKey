#![allow(clippy::doc_markdown)]

//! M3D-29 pipe security boundary for the Windows named-pipe transport.
//!
//! Three narrow, fail-closed mechanisms back the hardened transport in
//! `pipe_transport`:
//!
//! 1. An explicit DACL on every created pipe instance that grants access to
//!    the creating process's user SID only. The default process DACL (which
//!    would also admit administrators and local system) is never relied on.
//!    Because the DACL is explicit, other interactive users, `Everyone`,
//!    administrators, and `LOCAL SYSTEM` are denied by omission. Accurate
//!    admin statement: a Windows administrator can still take ownership of
//!    the pipe object (or enable `SeSecurityPrivilege`), rewrite the DACL,
//!    and connect; that is inherent Windows administrative power and is a
//!    documented residual threat, not an access-control bug.
//! 2. A narrow server-side peer verification: immediately after
//!    `ConnectNamedPipe`, the server impersonates the client at
//!    identification level only, reads the client token's user SID, compares
//!    it with its own SID, and always reverts before serving anything. Any
//!    failure (impersonation, token query, SID mismatch, revert) drops the
//!    connection without serving it. This is identity inspection, not
//!    cryptographic authentication, and is never claimed as such.
//! 3. Unpredictable per-lifecycle identity: 128-bit `BCryptGenRandom`
//!    nonces back generated pipe names and session ids, so a new server
//!    lifecycle always has a fresh identity and a stale identity authorizes
//!    nothing.
//!
//! PID, window handle, and pipe name alone are never trusted identity.

use std::fmt::Write as _;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, GENERIC_ALL, HANDLE, HLOCAL, LocalFree,
};
#[cfg(test)]
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SetEntriesInAclW, TRUSTEE_IS_SID,
    TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
#[cfg(test)]
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL_SIZE_INFORMATION, AclSizeInformation,
    DACL_SECURITY_INFORMATION, GetAce, GetAclInformation,
};
use windows::Win32::Security::{
    ACL, EqualSid, GetLengthSid, GetTokenInformation, InitializeSecurityDescriptor, IsValidSid,
    NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, RevertToSelf, SECURITY_ATTRIBUTES,
    SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
#[cfg(test)]
use windows::Win32::Security::{GetKernelObjectSecurity, GetSecurityDescriptorDacl};
use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
#[cfg(test)]
use windows::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};
use windows::core::PWSTR;

/// `SECURITY_DESCRIPTOR_REVISION` (the constant is not exported by the
/// windows crate in this version).
const SECURITY_DESCRIPTOR_REVISION_VALUE: u32 = 1;

/// Fail-closed pipe security errors. Every variant means the boundary was
/// not established or not proven; callers must refuse to proceed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PipeSecurityError {
    /// The creating process's user SID could not be read.
    CurrentUserSidUnavailable,
    /// The system-preferred RNG could not produce a nonce.
    RandomUnavailable,
    /// The explicit current-user-only DACL could not be constructed.
    DaclConstructionFailed,
    /// `ImpersonateNamedPipeClient` failed for a connected peer.
    PeerImpersonationFailed,
    /// The impersonated peer token could not be read.
    PeerTokenUnavailable,
    /// The peer token's user SID is not the current user's SID.
    PeerSidMismatch,
    /// `RevertToSelf` failed after inspection.
    RevertFailed,
    /// The DACL of an existing handle could not be inspected (Win32 error).
    #[cfg(test)]
    DaclInspectionFailed(u32),
}

/// A SID copied out of a token and owned by this struct.
pub(crate) struct OwnedSid {
    bytes: Vec<u8>,
}

impl OwnedSid {
    fn psid(&self) -> PSID {
        PSID(self.bytes.as_ptr().cast::<core::ffi::c_void>().cast_mut())
    }

    /// The `S-1-…` string form, for tests and sanitized diagnostics.
    #[cfg(test)]
    pub(crate) fn string_form(&self) -> Option<String> {
        let mut wide = PWSTR::null();
        if unsafe { ConvertSidToStringSidW(self.psid(), &raw mut wide) }.is_err() {
            return None;
        }
        if wide.is_null() {
            return None;
        }
        // Safety: ConvertSidToStringSidW returned a NUL-terminated
        // LocalAlloc'd string.
        let mut length = 0usize;
        while unsafe { *wide.0.add(length) } != 0 {
            length += 1;
        }
        let text = String::from_utf16(unsafe { core::slice::from_raw_parts(wide.0, length) }).ok();
        let _ = unsafe { LocalFree(Some(HLOCAL(wide.0.cast::<core::ffi::c_void>()))) };
        text
    }
}

/// Reads the user SID from an access token and copies it into an OwnedSid.
fn token_user_sid(token: HANDLE) -> Result<OwnedSid, PipeSecurityError> {
    let mut needed = 0u32;
    // The size probe always fails with ERROR_INSUFFICIENT_BUFFER; a zero
    // size means the token cannot be inspected.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &raw mut needed) };
    if needed == 0 {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    let mut buffer = vec![0u8; needed as usize];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast::<core::ffi::c_void>()),
            needed,
            &raw mut needed,
        )
    }
    .is_err()
    {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    // The Vec<u8> buffer has no struct alignment guarantee, so read the
    // TOKEN_USER header unaligned and copy the SID bytes out of it.
    let user = unsafe { core::ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
    if user.User.Sid.0.is_null() {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    let sid_length = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if sid_length == 0 || sid_length > buffer.len() {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    let bytes =
        unsafe { core::slice::from_raw_parts(user.User.Sid.0.cast::<u8>(), sid_length).to_vec() };
    let candidate = OwnedSid { bytes };
    if !unsafe { IsValidSid(candidate.psid()) }.as_bool() {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    Ok(candidate)
}

/// The creating process's current user SID.
pub(crate) fn current_user_sid() -> Result<OwnedSid, PipeSecurityError> {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) }.is_err() {
        return Err(PipeSecurityError::CurrentUserSidUnavailable);
    }
    let result = token_user_sid(token);
    let _ = unsafe { CloseHandle(token) };
    result
}

/// `SECURITY_ATTRIBUTES` carrying an explicit DACL that grants the current
/// user's SID full access and nothing else. The descriptor lives behind a
/// `Box` so its address stays stable when the guard moves into the listener
/// thread; the ACL is `LocalAlloc`'d by `SetEntriesInAclW` and freed on drop.
pub(crate) struct PipeSecurityAttributes {
    attributes: SECURITY_ATTRIBUTES,
    /// Keep-alive storage for the absolute security descriptor that
    /// `attributes.lpSecurityDescriptor` points into; boxed so the address
    /// is stable when the guard moves.
    _descriptor: Box<SECURITY_DESCRIPTOR>,
    acl: *mut ACL,
}

// Safety: the guard is moved exactly once into the listener thread before
// any use, and both pointers target heap storage that is not tied to the
// guard's stack address.
unsafe impl Send for PipeSecurityAttributes {}

impl PipeSecurityAttributes {
    /// Builds the current-user-only security attributes. Callers that fail
    /// to build these must not create the pipe with weaker security.
    pub(crate) fn current_user_only() -> Result<Self, PipeSecurityError> {
        let sid = current_user_sid()?;
        let entries = [EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL.0,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: core::ptr::null_mut(),
                MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_USER,
                ptstrName: PWSTR(sid.psid().0.cast::<u16>()),
            },
        }];
        let mut acl: *mut ACL = core::ptr::null_mut();
        // SetEntriesInAclW copies the trustee SID into the new ACL, so the
        // OwnedSid may be dropped after this call.
        let status = unsafe { SetEntriesInAclW(Some(&entries), None, &raw mut acl) };
        if status != ERROR_SUCCESS || acl.is_null() {
            return Err(PipeSecurityError::DaclConstructionFailed);
        }
        let mut descriptor = Box::new(unsafe { core::mem::zeroed::<SECURITY_DESCRIPTOR>() });
        let descriptor_ptr = PSECURITY_DESCRIPTOR(
            core::ptr::from_mut::<SECURITY_DESCRIPTOR>(&mut descriptor).cast::<core::ffi::c_void>(),
        );
        if unsafe {
            InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION_VALUE)
        }
        .is_err()
        {
            let _ = unsafe { LocalFree(Some(HLOCAL(acl.cast::<core::ffi::c_void>()))) };
            return Err(PipeSecurityError::DaclConstructionFailed);
        }
        if unsafe { SetSecurityDescriptorDacl(descriptor_ptr, true, Some(acl), false) }.is_err() {
            let _ = unsafe { LocalFree(Some(HLOCAL(acl.cast::<core::ffi::c_void>()))) };
            return Err(PipeSecurityError::DaclConstructionFailed);
        }
        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: u32::try_from(core::mem::size_of::<SECURITY_ATTRIBUTES>())
                    .expect("SECURITY_ATTRIBUTES size fits u32"),
                lpSecurityDescriptor: descriptor_ptr.0,
                bInheritHandle: windows::core::BOOL::from(false),
            },
            _descriptor: descriptor,
            acl,
        })
    }

    /// The pointer to pass as `lpSecurityAttributes`. Valid for as long as
    /// the guard is alive.
    pub(crate) fn as_ptr(&self) -> *const SECURITY_ATTRIBUTES {
        core::ptr::from_ref(&self.attributes)
    }
}

impl Drop for PipeSecurityAttributes {
    fn drop(&mut self) {
        if !self.acl.is_null() {
            let _ = unsafe { LocalFree(Some(HLOCAL(self.acl.cast::<core::ffi::c_void>()))) };
        }
    }
}

/// Verifies that the peer on the other end of a connected server-side pipe
/// handle is the current user, by impersonating the client at identification
/// level, reading its token user SID, comparing it with the server's own SID,
/// and always reverting before returning. Any failure is fail-closed: the
/// caller must drop the connection without serving it. This is SID identity
/// inspection, not cryptographic authentication.
pub(crate) fn verify_peer_is_current_user(pipe_handle: HANDLE) -> Result<(), PipeSecurityError> {
    let own = current_user_sid()?;
    if unsafe { ImpersonateNamedPipeClient(pipe_handle) }.is_err() {
        return Err(PipeSecurityError::PeerImpersonationFailed);
    }
    let inspected = (|| {
        let mut token = HANDLE::default();
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, false, &raw mut token) }
            .is_err()
        {
            return Err(PipeSecurityError::PeerTokenUnavailable);
        }
        let peer = token_user_sid(token);
        let _ = unsafe { CloseHandle(token) };
        let peer = peer.map_err(|_| PipeSecurityError::PeerTokenUnavailable)?;
        if unsafe { EqualSid(peer.psid(), own.psid()) }.is_err() {
            return Err(PipeSecurityError::PeerSidMismatch);
        }
        Ok(())
    })();
    if unsafe { RevertToSelf() }.is_err() {
        return Err(PipeSecurityError::RevertFailed);
    }
    inspected
}

/// Unpredictable lowercase hex nonce from the system-preferred RNG
/// (`BCryptGenRandom`), 2 characters per input byte.
pub(crate) fn random_nonce_hex(byte_length: usize) -> Result<String, PipeSecurityError> {
    let mut bytes = vec![0u8; byte_length];
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_err() {
        return Err(PipeSecurityError::RandomUnavailable);
    }
    let mut text = String::with_capacity(byte_length * 2);
    for byte in &bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// Facts extracted from a handle's DACL, used by the hardening tests and
/// sanitized diagnostics.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DaclFacts {
    /// Number of ACEs in the DACL.
    pub(crate) ace_count: usize,
    /// True when every ACE is an allow ACE whose trustee is exactly the
    /// current user's SID (no other grantees, no deny or exotic ACEs).
    pub(crate) all_allowed_aces_grant_current_user_only: bool,
    /// True when at least one deny ACE is present.
    pub(crate) has_deny_ace: bool,
}

/// Inspects a pipe handle's DACL and reports whether it grants access to
/// the current user only. Named pipes are queried as kernel objects
/// (`GetKernelObjectSecurity`); the returned descriptor is self-relative,
/// so the DACL is located with `GetSecurityDescriptorDacl`.
#[cfg(test)]
pub(crate) fn inspect_current_user_only_dacl(
    handle: HANDLE,
) -> Result<DaclFacts, PipeSecurityError> {
    let own = current_user_sid()?;
    let mut needed = 0u32;
    let _ = unsafe {
        GetKernelObjectSecurity(
            handle,
            DACL_SECURITY_INFORMATION.0,
            None,
            0,
            &raw mut needed,
        )
    };
    if needed == 0 {
        return Err(PipeSecurityError::DaclInspectionFailed(0));
    }
    let mut buffer = vec![0u8; needed as usize];
    let queried = unsafe {
        GetKernelObjectSecurity(
            handle,
            DACL_SECURITY_INFORMATION.0,
            Some(PSECURITY_DESCRIPTOR(
                buffer.as_mut_ptr().cast::<core::ffi::c_void>(),
            )),
            needed,
            &raw mut needed,
        )
    };
    if let Err(error) = queried {
        let code = u32::try_from(error.code().0 & 0xFFFF).unwrap_or(0);
        return Err(PipeSecurityError::DaclInspectionFailed(code));
    }
    let mut present = windows::core::BOOL::default();
    let mut defaulted = windows::core::BOOL::default();
    let mut dacl: *mut ACL = core::ptr::null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(
            PSECURITY_DESCRIPTOR(buffer.as_ptr() as *mut core::ffi::c_void),
            &raw mut present,
            &raw mut dacl,
            &raw mut defaulted,
        )
    }
    .is_err()
        || !present.as_bool()
        || dacl.is_null()
    {
        return Err(PipeSecurityError::DaclInspectionFailed(0));
    }
    let mut size = ACL_SIZE_INFORMATION::default();
    let sized = unsafe {
        GetAclInformation(
            dacl,
            (&raw mut size).cast::<core::ffi::c_void>(),
            u32::try_from(core::mem::size_of::<ACL_SIZE_INFORMATION>())
                .expect("ACL_SIZE_INFORMATION size fits u32"),
            AclSizeInformation,
        )
    }
    .is_ok();
    let mut facts = DaclFacts {
        ace_count: 0,
        all_allowed_aces_grant_current_user_only: sized,
        has_deny_ace: false,
    };
    if sized {
        facts.ace_count = size.AceCount as usize;
        for index in 0..size.AceCount {
            let mut ace: *mut core::ffi::c_void = core::ptr::null_mut();
            if unsafe { GetAce(dacl, index, &raw mut ace) }.is_err() || ace.is_null() {
                facts.all_allowed_aces_grant_current_user_only = false;
                continue;
            }
            let header = unsafe { &*(ace.cast::<ACE_HEADER>()) };
            if u32::from(header.AceType) == ACCESS_ALLOWED_ACE_TYPE {
                let allowed = unsafe { &*(ace.cast::<ACCESS_ALLOWED_ACE>()) };
                let sid = PSID(
                    core::ptr::from_ref(&allowed.SidStart)
                        .cast::<core::ffi::c_void>()
                        .cast_mut(),
                );
                if unsafe { EqualSid(sid, own.psid()) }.is_err() {
                    facts.all_allowed_aces_grant_current_user_only = false;
                }
            } else {
                // Deny, callback, compound, or object ACEs all violate the
                // current-user-only expectation.
                facts.all_allowed_aces_grant_current_user_only = false;
                if u32::from(header.AceType) == ACCESS_DENIED_ACE_TYPE {
                    facts.has_deny_ace = true;
                }
            }
        }
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_sid_is_valid_stable_and_shaped() {
        let first = current_user_sid().expect("current user sid");
        let second = current_user_sid().expect("second read");
        assert!(unsafe { IsValidSid(first.psid()) }.as_bool());
        assert!(unsafe { EqualSid(first.psid(), second.psid()) }.is_ok());
        let text = first.string_form().expect("string form");
        assert!(text.starts_with("S-1-"), "sid={text}");
    }

    #[test]
    fn nonces_are_full_length_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..64 {
            let nonce = random_nonce_hex(16).expect("nonce");
            assert_eq!(nonce.len(), 32);
            assert!(
                nonce
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "nonce={nonce}"
            );
            assert!(seen.insert(nonce));
        }
    }

    #[test]
    fn security_attributes_build_and_free_cleanly() {
        let guard = PipeSecurityAttributes::current_user_only().expect("attributes");
        assert!(!guard.as_ptr().is_null());
        // Building twice proves no shared state leaks between instances;
        // both guards free their ACLs on drop.
        let _again = PipeSecurityAttributes::current_user_only().expect("second attributes");
    }
}
