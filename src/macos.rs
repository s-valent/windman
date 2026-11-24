use std::path::Path;

use objc2::{AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSApplication, NSApplicationActivationPolicy, NSAutoresizingMaskOptions,
    NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSView, NSVisualEffectBlendingMode,
    NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    NSWorkspace,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString, ns_string};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn configure_window(window: &gpui::Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let view = unsafe { &*(appkit.ns_view.as_ptr() as *const NSView) };
    let Some(native_window) = view.window() else {
        return;
    };
    let Some(content) = native_window.contentView() else {
        return;
    };
    let backdrop =
        NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(main_thread), content.bounds());
    backdrop.setMaterial(NSVisualEffectMaterial::HUDWindow);
    backdrop.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    backdrop.setState(NSVisualEffectState::Active);
    backdrop.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    content.addSubview_positioned_relativeTo(&backdrop, NSWindowOrderingMode::Below, Some(view));
    unsafe {
        let _: () = objc2::msg_send![&content, setWantsLayer: objc2::runtime::Bool::YES];
        let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![&content, layer];
        if !layer.is_null() {
            let _: () = objc2::msg_send![layer, setCornerRadius: 8.0f64];
            let _: () = objc2::msg_send![layer, setMasksToBounds: objc2::runtime::Bool::YES];
        }
    }
}

pub fn accessory_app() {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let _ = NSApplication::sharedApplication(main_thread)
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

pub fn dark_appearance() {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let application = NSApplication::sharedApplication(main_thread);
    let name = ns_string!("NSAppearanceNameDarkAqua");
    if application
        .appearance()
        .is_some_and(|appearance| &*appearance.name() == name)
    {
        return;
    }
    if let Some(appearance) = NSAppearance::appearanceNamed(name) {
        application.setAppearance(Some(&appearance));
    }
}

pub fn app_icon(path: &Path, output: &Path) -> bool {
    let Some(_main_thread) = MainThreadMarker::new() else {
        return false;
    };
    let Some(path) = path.to_str() else {
        return false;
    };
    let Some(output) = output.to_str() else {
        return false;
    };
    let workspace = NSWorkspace::sharedWorkspace();
    let image = workspace.iconForFile(&NSString::from_str(path));
    let target = NSImage::initWithSize(NSImage::alloc(), NSSize::new(64., 64.));
    #[allow(deprecated)]
    {
        target.lockFocus();
        image.drawInRect(NSRect::new(NSPoint::new(0., 0.), NSSize::new(64., 64.)));
        target.unlockFocus();
    }
    let Some(tiff) = target.TIFFRepresentation() else {
        return false;
    };
    let Some(representation) = NSBitmapImageRep::imageRepWithData(&tiff) else {
        return false;
    };
    let Some(data) = (unsafe {
        representation
            .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }) else {
        return false;
    };
    data.writeToFile_atomically(&NSString::from_str(output), true)
}
