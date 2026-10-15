use super::*;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::Security::{
    AclSizeInformation,
    Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT},
    GetAce, GetAclInformation, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE,
    ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
};

#[test]
fn pipe_roundtrip_reuse_first_instance_and_slow_client_deadline() {
    let expected = Response {
        version: 1,
        status: Status::Ok,
        credential: None,
        matches: vec![Match {
            id: "synthetic".into(),
            title: "Synthetic test login".into(),
            username: "test@example.com".into(),
            url: "https://example.com".into(),
        }],
    };
    let value = expected.clone();
    let _guard = start_server(move |_| value.clone()).unwrap();
    assert!(start_server(|_| Response::empty(Status::Error)).is_err());
    let request = Request::Match {
        version: 1,
        origin: "https://example.com".into(),
    };
    for index in 0..20 {
        assert_eq!(
            request_desktop(&request).unwrap_or_else(|error| panic!("request {index}: {error}")),
            expected
        );
    }
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let sid = current_sid().unwrap();
    runtime.block_on(async {
        let _idle_client = timeout(DEADLINE, async {
            loop {
                match ClientOptions::new().open(pipe_name(&sid)) {
                    Ok(pipe) => break pipe,
                    Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                        sleep(Duration::from_millis(10)).await
                    }
                    Err(error) => panic!("Couldn't connect synthetic idle client: {error}"),
                }
            }
        })
        .await
        .unwrap();
        sleep(DEADLINE + Duration::from_millis(100)).await;
    });
    assert_eq!(request_desktop(&request).unwrap(), expected);
    assert!(start_server(|_| Response::empty(Status::Error)).is_err());
}

#[test]
fn pipe_dacl_is_protected_and_grants_only_user_and_system() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let _entered = runtime.enter();
    let sid = current_sid().unwrap();
    let name = format!("{}-acl-test-{}", pipe_name(&sid), std::process::id());
    let server = create_server(&sid, &name, true).unwrap();
    unsafe {
        let mut dacl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        assert_eq!(
            GetSecurityInfo(
                server.as_raw_handle(),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut dacl,
                ptr::null_mut(),
                &mut descriptor
            ),
            0
        );
        let _descriptor = LocalAllocation(descriptor);
        assert!(!dacl.is_null());
        let mut control = 0;
        let mut revision = 0;
        assert_ne!(
            GetSecurityDescriptorControl(descriptor, &mut control, &mut revision),
            0
        );
        assert_ne!(control & SE_DACL_PROTECTED, 0);
        let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
        assert_ne!(
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation
            ),
            0
        );
        assert_eq!(info.AceCount, 2);
        let mut actual = Vec::new();
        for index in 0..info.AceCount {
            let mut raw = ptr::null_mut();
            assert_ne!(GetAce(dacl, index, &mut raw), 0);
            let ace = &*raw.cast::<ACCESS_ALLOWED_ACE>();
            assert_eq!(ace.Header.AceType, 0); // ACCESS_ALLOWED_ACE_TYPE
            let mut text = ptr::null_mut();
            assert_ne!(
                ConvertSidToStringSidW((&ace.SidStart as *const u32).cast_mut().cast(), &mut text),
                0
            );
            let _text = LocalAllocation(text.cast());
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            actual.push(String::from_utf16(std::slice::from_raw_parts(text, length)).unwrap());
        }
        actual.sort();
        let mut expected = vec![sid, "S-1-5-18".to_owned()];
        expected.sort();
        assert_eq!(actual, expected);
    }
}
