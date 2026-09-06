#pragma once

#include <lvgl.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*wm_action_cb_t)(lv_obj_t *obj, void *user_data);

void wm_init(void);

lv_obj_t *wm_create_window(lv_obj_t *parent, const char *title);
lv_obj_t *wm_add_text(lv_obj_t *parent, const char *text);
lv_obj_t *wm_add_button(lv_obj_t *parent, const char *label, int32_t w,
                        int32_t h);

void wm_on_tap(lv_obj_t *obj, wm_action_cb_t cb, void *user_data);
void wm_on_hold(lv_obj_t *obj, wm_action_cb_t cb, void *user_data);

void wm_toast(const char *message, uint32_t duration_ms);

#ifdef __cplusplus
}
#endif
