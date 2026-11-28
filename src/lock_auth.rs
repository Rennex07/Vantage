use std::{
    ffi::{CStr, c_char, c_int, c_void},
    fs::File,
    io::{self, Read, Write},
    os::fd::FromRawFd,
    process::{Child, Command, Stdio},
    ptr,
};

pub const SERVICE_PATH: &str = "/etc/pam.d/vantage-lock";
const PASSWORD_CAPACITY: usize = 512;
const PAM_SUCCESS: c_int = 0;
const PAM_SYSTEM_ERR: c_int = 4;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_DISALLOW_NULL_AUTHTOK: c_int = 1;
const PAM_USER: c_int = 2;

#[derive(Default)]
pub struct Password {
    // Keep the allocation fixed so typing never leaves a previous copy behind.
    bytes: Vec<u8>,
    characters: usize,
}

impl Password {
    pub fn push(&mut self, character: char) {
        if self.bytes.capacity() == 0 {
            self.bytes.reserve_exact(PASSWORD_CAPACITY);
        }
        if self.bytes.len() + character.len_utf8() < PASSWORD_CAPACITY {
            let mut encoded = [0_u8; 4];
            self.bytes
                .extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            erase(&mut encoded);
            self.characters += 1;
        }
    }

    pub fn pop(&mut self) {
        if let Some(index) = self.bytes.iter().rposition(|byte| byte & 0xc0 != 0x80) {
            erase(&mut self.bytes[index..]);
            self.bytes.truncate(index);
            self.characters -= 1;
        }
    }

    pub fn clear(&mut self) {
        erase(&mut self.bytes);
        self.bytes.clear();
        self.characters = 0;
    }

    pub fn len(&self) -> usize {
        self.characters
    }

    pub fn submit(&mut self) -> io::Result<Child> {
        let result = self.start_helper();
        self.clear();
        result
    }

    fn start_helper(&self) -> io::Result<Child> {
        let mut child = Command::new("/proc/self/exe")
            .arg("--authenticate-lock")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut input = child.stdin.take().expect("piped authentication input");
        if let Err(error) = input.write_all(&self.bytes) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(child)
    }
}

impl Drop for Password {
    fn drop(&mut self) {
        self.clear();
    }
}

fn erase(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: Each byte belongs to this exclusive slice.
        unsafe { ptr::write_volatile(byte, 0) };
    }
}

#[repr(C)]
struct PamMessage {
    style: c_int,
    text: *const c_char,
}

#[repr(C)]
struct PamResponse {
    response: *mut c_char,
    code: c_int,
}

#[repr(C)]
struct PamConversation {
    callback: unsafe extern "C" fn(
        c_int,
        *const *const PamMessage,
        *mut *mut PamResponse,
        *mut c_void,
    ) -> c_int,
    data: *mut c_void,
}

struct Conversation<'a> {
    username: &'a CStr,
    password: &'a CStr,
    supplied_password: bool,
}

#[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conversation: *const PamConversation,
        handle: *mut *mut c_void,
    ) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_get_item(handle: *const c_void, kind: c_int, item: *mut *const c_void) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *const *const PamMessage,
    responses: *mut *mut PamResponse,
    data: *mut c_void,
) -> c_int {
    if !(1..=32).contains(&count) || messages.is_null() || responses.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: PAM supplies count message pointers and the conversation we passed to pam_start.
    // Response pointers are allocated with libc as PAM releases them after a successful callback.
    unsafe {
        *responses = ptr::null_mut();
        let conversation = &mut *data.cast::<Conversation<'_>>();
        let messages = std::slice::from_raw_parts(messages, count as usize);
        let result = libc::calloc(messages.len(), size_of::<PamResponse>()).cast::<PamResponse>();
        if result.is_null() {
            return PAM_BUF_ERR;
        }
        let result_slice = std::slice::from_raw_parts_mut(result, messages.len());
        let status = fill_responses(conversation, messages, result_slice);
        if status == PAM_SUCCESS {
            *responses = result;
        } else {
            for entry in result_slice {
                if !entry.response.is_null() {
                    let len = CStr::from_ptr(entry.response).to_bytes().len();
                    erase(std::slice::from_raw_parts_mut(entry.response.cast(), len));
                    libc::free(entry.response.cast());
                }
            }
            libc::free(result.cast());
        }
        status
    }
}

fn fill_responses(
    conversation: &mut Conversation<'_>,
    messages: &[*const PamMessage],
    responses: &mut [PamResponse],
) -> c_int {
    for (&message, response) in messages.iter().zip(responses) {
        // SAFETY: PAM owns these message records for the duration of the callback.
        let Some(message) = (unsafe { message.as_ref() }) else {
            return PAM_CONV_ERR;
        };
        let text = match message.style {
            1 if !conversation.supplied_password => {
                conversation.supplied_password = true;
                conversation.password
            }
            2 => conversation.username,
            3 | 4 => continue,
            _ => return PAM_CONV_ERR,
        };
        // SAFETY: text points to an existing NUL-terminated string.
        response.response = unsafe { libc::strdup(text.as_ptr()) };
        if response.response.is_null() {
            return PAM_BUF_ERR;
        }
    }
    PAM_SUCCESS
}

pub fn run_helper() -> i32 {
    // SAFETY: This is a standalone process. Its only input is the private password pipe.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    let mut bytes = Vec::with_capacity(PASSWORD_CAPACITY + 1);
    // SAFETY: This helper owns stdin and uses it exclusively for the request.
    let input = unsafe { File::from_raw_fd(libc::STDIN_FILENO) };
    let read = input.take(PASSWORD_CAPACITY as u64).read_to_end(&mut bytes);
    let result = if read.is_err() || bytes.is_empty() || bytes.len() >= PASSWORD_CAPACITY {
        1
    } else {
        bytes.push(0);
        match CStr::from_bytes_with_nul(&bytes) {
            Ok(password) => authenticate(password),
            Err(_) => 1,
        }
    };
    erase(&mut bytes);
    result
}

fn authenticate(password: &CStr) -> i32 {
    // SAFETY: The standalone helper is single-threaded and immediately copies the NSS result.
    // The identity comes from the process UID rather than any environment variable.
    unsafe {
        let record = libc::getpwuid(libc::getuid());
        if record.is_null() || (*record).pw_name.is_null() {
            return 2;
        }
        let username = CStr::from_ptr((*record).pw_name).to_owned();
        let mut data = Conversation {
            username: &username,
            password,
            supplied_password: false,
        };
        let conversation = PamConversation {
            callback: converse,
            data: ptr::from_mut(&mut data).cast(),
        };
        let mut handle = ptr::null_mut();
        let start = pam_start(
            c"vantage-lock".as_ptr(),
            username.as_ptr(),
            &conversation,
            &mut handle,
        );
        if start != PAM_SUCCESS {
            return 2;
        }
        let flags = PAM_DISALLOW_NULL_AUTHTOK;
        let mut status = pam_authenticate(handle, flags);
        if status == PAM_SUCCESS {
            status = pam_acct_mgmt(handle, flags);
        }
        let mut mapped_user = ptr::null();
        let same_user = status == PAM_SUCCESS
            && pam_get_item(handle, PAM_USER, &mut mapped_user) == PAM_SUCCESS
            && !mapped_user.is_null()
            && CStr::from_ptr(mapped_user.cast()) == username.as_c_str();
        let end = pam_end(handle, status);
        match (status, end, same_user, data.supplied_password) {
            (PAM_SUCCESS, PAM_SUCCESS, true, true) => 0,
            (PAM_SYSTEM_ERR | PAM_BUF_ERR | PAM_CONV_ERR, _, _, _) => 2,
            (_, end, _, _) if end != PAM_SUCCESS => 2,
            _ => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_multibyte_input_preserves_the_remaining_character() {
        let mut password = Password::default();
        password.push('a');
        password.push('🌙');
        password.pop();
        assert_eq!(password.len(), 1);
        assert_eq!(password.bytes, b"a");
        password.clear();
        assert_eq!(password.len(), 0);
        assert!(password.bytes.is_empty());
    }
}
