// Neither WebKitGTK (Linux) nor WebView2 (Windows) grant getUserMedia by
// default in an embedded webview: WebKitGTK silently denies every request
// unless the host connects to `permission-request`, and WebView2 needs an
// explicit `PermissionRequested` handler to auto-allow instead of leaving
// the mic permanently blocked. macOS/iOS grant via the audio-input
// entitlement instead, so there's nothing to wire up there.
use tauri::WebviewWindow;

pub fn allow_media_permissions(window: &WebviewWindow) {
    let _ = window.with_webview(|_webview| {
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd"
        ))]
        {
            use webkit2gtk::{PermissionRequestExt, WebViewExt};
            _webview.inner().connect_permission_request(|_, request| {
                request.allow();
                true
            });
        }

        #[cfg(windows)]
        {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2PermissionRequestedEventArgs, COREWEBVIEW2_PERMISSION_STATE_ALLOW,
            };
            use webview2_com::PermissionRequestedEventHandler;

            if let Ok(core) = unsafe { _webview.controller().CoreWebView2() } {
                let handler = PermissionRequestedEventHandler::create(Box::new(
                    move |_sender, args: Option<ICoreWebView2PermissionRequestedEventArgs>| {
                        if let Some(args) = args {
                            unsafe { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)? };
                        }
                        Ok(())
                    },
                ));
                let mut token = windows::Win32::System::WinRT::EventRegistrationToken::default();
                unsafe {
                    let _ = core.add_PermissionRequested(&handler, &mut token);
                }
            }
        }
    });
}
