#include "WindowServer.h"
#include <Carbon/Carbon.h>
#include <dlfcn.h>
#include <pthread.h>
#include <string.h>
#include <unistd.h>

typedef int32_t ConnectionID;

static ConnectionID (*main_connection)(void);
static int32_t (*find_window)(ConnectionID, int, int, int, CGPoint *, CGPoint *,
                              CGWindowID *, ConnectionID *);
static AXError (*accessibility_window_id)(AXUIElementRef, CGWindowID *);
static CGError (*window_owner)(ConnectionID, CGWindowID, ConnectionID *);
static CGError (*connection_process)(ConnectionID, ProcessSerialNumber *);
static OSStatus (*process_for_pid)(pid_t, ProcessSerialNumber *);
static CGError (*set_front_process)(ProcessSerialNumber *, CGWindowID, uint32_t);
static CGError (*post_event)(ProcessSerialNumber *, uint8_t *);
static CFArrayRef (*copy_display_spaces)(ConnectionID);
static CFArrayRef (*copy_window_spaces)(ConnectionID, int, CFArrayRef);
static bool (*display_is_animating)(ConnectionID, CFStringRef);
static const char *initialization_error;
static pthread_once_t initialization_once = PTHREAD_ONCE_INIT;

static void initialize_symbols(void) {
    void *sky_light = dlopen(
        "/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight",
        RTLD_LAZY | RTLD_LOCAL);
    void *accessibility = dlopen(
        "/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices",
        RTLD_LAZY | RTLD_LOCAL);
    if (!sky_light || !accessibility) {
        initialization_error = "Could not load macOS window-management frameworks.";
        return;
    }

    main_connection = dlsym(sky_light, "SLSMainConnectionID");
    find_window = dlsym(sky_light, "SLSFindWindowAndOwner");
    window_owner = dlsym(sky_light, "SLSGetWindowOwner");
    connection_process = dlsym(sky_light, "SLSGetConnectionPSN");
    set_front_process = dlsym(sky_light, "_SLPSSetFrontProcessWithOptions");
    post_event = dlsym(sky_light, "SLPSPostEventRecordTo");
    accessibility_window_id = dlsym(accessibility, "_AXUIElementGetWindow");
    process_for_pid = dlsym(accessibility, "GetProcessForPID");
    copy_display_spaces = dlsym(sky_light, "SLSCopyManagedDisplaySpaces");
    copy_window_spaces = dlsym(sky_light, "SLSCopySpacesForWindows");
    display_is_animating = dlsym(sky_light, "SLSManagedDisplayIsAnimating");

    if (!main_connection || !find_window || !window_owner || !connection_process ||
        !set_front_process || !post_event || !accessibility_window_id ||
        !copy_display_spaces || !copy_window_spaces || !display_is_animating) {
        initialization_error = "Required private APIs are unavailable on this macOS version.";
    }
}

const char *wm_initialize(void) {
    pthread_once(&initialization_once, initialize_symbols);
    return initialization_error;
}

bool wm_request_accessibility(void) {
    const void *keys[] = {kAXTrustedCheckOptionPrompt};
    const void *values[] = {kCFBooleanTrue};
    CFDictionaryRef options = CFDictionaryCreate(
        kCFAllocatorDefault, keys, values, 1,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    bool trusted = AXIsProcessTrustedWithOptions(options);
    CFRelease(options);
    return trusted;
}

CFArrayRef wm_copy_active_spaces(bool *animating) {
    *animating = true;
    if (wm_initialize()) return NULL;
    ConnectionID connection = main_connection();
    CFArrayRef displays = copy_display_spaces(connection);
    if (!displays) return NULL;
    CFMutableArrayRef spaces = CFArrayCreateMutable(
        kCFAllocatorDefault, 0, &kCFTypeArrayCallBacks);
    bool transitioning = false;
    for (CFIndex index = 0; index < CFArrayGetCount(displays); ++index) {
        CFDictionaryRef display = CFArrayGetValueAtIndex(displays, index);
        if (CFGetTypeID(display) != CFDictionaryGetTypeID()) continue;
        CFStringRef identifier = CFDictionaryGetValue(display, CFSTR("Display Identifier"));
        if (identifier && CFGetTypeID(identifier) == CFStringGetTypeID()) {
            transitioning |= display_is_animating(connection, identifier);
        }
        CFDictionaryRef current = CFDictionaryGetValue(display, CFSTR("Current Space"));
        if (!current || CFGetTypeID(current) != CFDictionaryGetTypeID()) continue;
        CFNumberRef space = CFDictionaryGetValue(current, CFSTR("id64"));
        if (space && CFGetTypeID(space) == CFNumberGetTypeID()) {
            CFArrayAppendValue(spaces, space);
        }
    }
    CFRelease(displays);
    *animating = transitioning;
    return spaces;
}

CFArrayRef wm_copy_window_spaces(CGWindowID window) {
    if (wm_initialize()) return NULL;
    int64_t identifier = window;
    CFNumberRef number = CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt64Type, &identifier);
    const void *values[] = {number};
    CFArrayRef windows = CFArrayCreate(kCFAllocatorDefault, values, 1, &kCFTypeArrayCallBacks);
    CFArrayRef spaces = copy_window_spaces(main_connection(), 7, windows);
    CFRelease(windows);
    CFRelease(number);
    return spaces;
}

CGWindowID wm_window_at_point(CGPoint point) {
    if (wm_initialize()) return 0;
    CGPoint window_point = CGPointZero;
    CGWindowID window = 0;
    ConnectionID owner = 0;
    int32_t result = find_window(main_connection(), 0, 1, 0, &point,
                                 &window_point, &window, &owner);
    return result == 0 ? window : 0;
}

CGWindowID wm_accessibility_window_id(AXUIElementRef element) {
    if (wm_initialize()) return 0;
    CGWindowID window = 0;
    return accessibility_window_id(element, &window) == kAXErrorSuccess ? window : 0;
}

static CGError process_for_window(CGWindowID window, ProcessSerialNumber *process) {
    ConnectionID owner = 0;
    CGError result = window_owner(main_connection(), window, &owner);
    return result == kCGErrorSuccess ? connection_process(owner, process) : result;
}

static CGError change_key_window(ProcessSerialNumber *process, CGWindowID window) {
    uint8_t event[0xf8] = {0};
    event[0x04] = sizeof(event);
    event[0x3a] = 0x10;
    memset(event + 0x20, 0xff, 0x10);
    memcpy(event + 0x3c, &window, sizeof(window));
    event[0x08] = 0x01;
    CGError result = post_event(process, event);
    if (result != kCGErrorSuccess) return result;
    event[0x08] = 0x02;
    return post_event(process, event);
}

int32_t wm_focus_process(int32_t process_id) {
    if (wm_initialize() || !process_for_pid) return kCGErrorNotImplemented;
    ProcessSerialNumber process = {0};
    OSStatus result = process_for_pid(process_id, &process);
    if (result != noErr) return result;
    return set_front_process(&process, 0, 0x200);
}

int32_t wm_focus_window(CGWindowID window, CGWindowID previous_window) {
    if (wm_initialize()) return kCGErrorNotImplemented;
    ProcessSerialNumber process = {0};
    CGError result = process_for_window(window, &process);
    if (result != kCGErrorSuccess) return result;

    ProcessSerialNumber previous_process = {0};
    if (previous_window && previous_window != window &&
        process_for_window(previous_window, &previous_process) == kCGErrorSuccess &&
        previous_process.highLongOfPSN == process.highLongOfPSN &&
        previous_process.lowLongOfPSN == process.lowLongOfPSN) {
        uint8_t event[0xf8] = {0};
        event[0x04] = sizeof(event);
        event[0x08] = 0x0d;
        event[0x8a] = 0x02;
        memcpy(event + 0x3c, &previous_window, sizeof(previous_window));
        result = post_event(&process, event);
        if (result != kCGErrorSuccess) return result;
        usleep(40000);
        event[0x8a] = 0x01;
        memcpy(event + 0x3c, &window, sizeof(window));
        result = post_event(&process, event);
        if (result != kCGErrorSuccess) return result;
    }

    result = set_front_process(&process, window, 0x200);
    return result == kCGErrorSuccess ? change_key_window(&process, window) : result;
}
