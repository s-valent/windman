#pragma once

#include <ApplicationServices/ApplicationServices.h>
#include <stdint.h>

const char *wm_initialize(void);
bool wm_request_accessibility(void);
CFArrayRef wm_copy_active_spaces(bool *animating) CF_RETURNS_RETAINED;
CFArrayRef wm_copy_window_spaces(CGWindowID window) CF_RETURNS_RETAINED;
CGWindowID wm_window_at_point(CGPoint point);
CGWindowID wm_accessibility_window_id(AXUIElementRef element);
int32_t wm_focus_window(CGWindowID window, CGWindowID previous_window);
int32_t wm_focus_process(int32_t process_id);
