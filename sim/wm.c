#include "wm.h"
#include "wm_anim.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define APP_GRID_COLS 3

static lv_obj_t *main_screen = NULL;
static lv_obj_t *status_bar = NULL;
static lv_obj_t *app_container = NULL;
static lv_obj_t *nav_bar = NULL;
static lv_obj_t *home_view = NULL;
static lv_obj_t *current_app_view = NULL;

typedef struct {
  wm_action_cb_t tap_cb;
  wm_action_cb_t hold_cb;
  void *user_data;
} wm_event_ctx_t;

static void global_event_dispatcher(lv_event_t *e) {
  lv_event_code_t code = lv_event_get_code(e);
  lv_obj_t *target = lv_event_get_target(e);
  wm_event_ctx_t *ctx = (wm_event_ctx_t *)lv_event_get_user_data(e);

  if (!ctx)
    return;

  if (code == LV_EVENT_CLICKED && ctx->tap_cb) {
    ctx->tap_cb(target, ctx->user_data);
  } else if (code == LV_EVENT_LONG_PRESSED && ctx->hold_cb) {
    ctx->hold_cb(target, ctx->user_data);
  }
}

static wm_event_ctx_t *get_or_create_ctx(lv_obj_t *obj) {
  wm_event_ctx_t *ctx = (wm_event_ctx_t *)lv_obj_get_user_data(obj);
  if (!ctx) {
    ctx = (wm_event_ctx_t *)calloc(1, sizeof(wm_event_ctx_t));
    lv_obj_set_user_data(obj, ctx);
    lv_obj_add_event_cb(obj, global_event_dispatcher, LV_EVENT_ALL, ctx);
  }
  return ctx;
}

void wm_on_tap(lv_obj_t *obj, wm_action_cb_t cb, void *user_data) {
  wm_event_ctx_t *ctx = get_or_create_ctx(obj);
  ctx->tap_cb = cb;
  ctx->user_data = user_data;
}

void wm_on_hold(lv_obj_t *obj, wm_action_cb_t cb, void *user_data) {
  wm_event_ctx_t *ctx = get_or_create_ctx(obj);
  ctx->hold_cb = cb;
  ctx->user_data = user_data;
}

lv_obj_t *wm_create_window(lv_obj_t *parent, const char *title) {
  lv_obj_t *target_parent = parent ? parent : app_container;
  lv_obj_t *win = lv_obj_create(target_parent);
  lv_obj_remove_style_all(win);
  lv_obj_set_size(win, 320, 426);
  lv_obj_set_style_bg_color(win, lv_color_hex(0x121212), 0);
  lv_obj_set_style_bg_opa(win, LV_OPA_COVER, 0);
  lv_obj_set_flex_flow(win, LV_FLEX_FLOW_COLUMN);
  lv_obj_set_flex_align(win, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER,
                        LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_all(win, 8, 0);
  lv_obj_set_style_pad_gap(win, 8, 0);

  if (title) {
    wm_add_text(win, title);
  }
  return win;
}

lv_obj_t *wm_add_text(lv_obj_t *parent, const char *text) {
  lv_obj_t *lbl = lv_label_create(parent);
  lv_label_set_text(lbl, text);
  lv_obj_set_style_text_color(lbl, lv_color_hex(0xFFFFFF), 0);
  return lbl;
}

lv_obj_t *wm_add_button(lv_obj_t *parent, const char *label, int32_t w,
                        int32_t h) {
  lv_obj_t *btn = lv_btn_create(parent);
  lv_obj_set_size(btn, w, h);
  lv_obj_set_style_bg_color(btn, lv_color_hex(0x00E5FF), 0);
  lv_obj_set_style_bg_color(btn, lv_color_hex(0x0088A3), LV_STATE_PRESSED);

  lv_obj_t *lbl = lv_label_create(btn);
  lv_label_set_text(lbl, label);
  lv_obj_set_style_text_color(lbl, lv_color_hex(0x000000), 0);
  lv_obj_center(lbl);
  return btn;
}

static void toast_timer_cb(lv_timer_t *timer) {
  lv_obj_t *toast = (lv_obj_t *)lv_timer_get_user_data(timer);
  if (toast && lv_obj_is_valid(toast)) {
    lv_obj_del(toast);
  }
}

void wm_toast(const char *message, uint32_t duration_ms) {
  lv_obj_t *toast = lv_obj_create(lv_layer_top());
  lv_obj_set_size(toast, 260, 36);
  lv_obj_align(toast, LV_ALIGN_TOP_MID, 0, 28);

  lv_obj_set_style_bg_color(toast, lv_color_hex(0x222222), 0);
  lv_obj_set_style_border_color(toast, lv_color_hex(0x00E5FF), 0);
  lv_obj_set_style_border_width(toast, 1, 0);
  lv_obj_set_style_radius(toast, 18, 0);
  lv_obj_clear_flag(toast, LV_OBJ_FLAG_SCROLLABLE);

  lv_obj_t *lbl = lv_label_create(toast);
  lv_label_set_text(lbl, message);
  lv_obj_set_style_text_color(lbl, lv_color_hex(0xFFFFFF), 0);
  lv_obj_center(lbl);

  lv_timer_create(toast_timer_cb, duration_ms, toast);
}

static lv_obj_t *add_grid_app_item(lv_obj_t *parent, const char *title,
                                   const char *symbol) {
  lv_obj_t *cell = lv_obj_create(parent);
  lv_obj_remove_style_all(cell);
  lv_obj_set_size(cell, 68, 75);
  lv_obj_set_flex_flow(cell, LV_FLEX_FLOW_COLUMN);
  lv_obj_set_flex_align(cell, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER,
                        LV_FLEX_ALIGN_CENTER);
  lv_obj_set_style_pad_gap(cell, 4, 0);

  lv_obj_t *btn = lv_btn_create(cell);
  lv_obj_set_size(btn, 52, 52);
  lv_obj_set_style_radius(btn, 12, 0);
  lv_obj_set_style_bg_color(btn, lv_color_hex(0x00E5FF), 0);
  lv_obj_set_style_bg_color(btn, lv_color_hex(0x0088A3), LV_STATE_PRESSED);

  lv_obj_t *icon_lbl = lv_label_create(btn);
  lv_label_set_text(icon_lbl, symbol ? symbol : LV_SYMBOL_FILE);
  lv_obj_center(icon_lbl);

  lv_obj_t *name_lbl = lv_label_create(cell);
  lv_label_set_text(name_lbl, title);
  lv_obj_set_style_text_color(name_lbl, lv_color_hex(0xFFFFFF), 0);
  lv_obj_set_style_text_font(name_lbl, &lv_font_montserrat_10, 0);

  return btn;
}

static void close_app(void);

static void app_back_cb(lv_obj_t *obj, void *user_data) { close_app(); }
static void app_home_cb(lv_obj_t *obj, void *user_data) { close_app(); }
static void app_recents_cb(lv_obj_t *obj, void *user_data) {
  wm_toast("Recents (not implemented in sim)", 1500);
}

static void app_hold_cb(lv_obj_t *obj, void *user_data) {
  const char *name = (const char *)user_data;
  char buf[64];
  snprintf(buf, sizeof(buf), "Long press on %s", name);
  wm_toast(buf, 1500);
}

static void open_app(lv_obj_t *src, const char *title, const char *symbol) {
  if (current_app_view) {
    lv_obj_del(current_app_view);
    current_app_view = NULL;
  }

  /* Hide the home grid behind the new app window. */
  if (home_view) {
    lv_obj_add_flag(home_view, LV_OBJ_FLAG_HIDDEN);
  }

  lv_obj_t *app_view = wm_create_window(app_container, title);
  wm_add_text(app_view, "This is the simulated app window.");

  lv_obj_t *btn = wm_add_button(app_view, "Back", 120, 40);
  wm_on_tap(btn, app_back_cb, NULL);

  /* Make sure layout is computed so coordinates are correct. */
  lv_obj_update_layout(app_view);

  /* Animate from the tapped icon into the new window. */
  wm_animate_launch(src, app_view);

  current_app_view = app_view;
}

static void close_app(void) {
  if (!current_app_view)
    return;
  lv_obj_del(current_app_view);
  current_app_view = NULL;

  if (home_view) {
    lv_obj_clear_flag(home_view, LV_OBJ_FLAG_HIDDEN);
    lv_obj_move_foreground(home_view);
  }
}

static void wifi_tap_cb(lv_obj_t *obj, void *user_data) {
  open_app(obj, "Wi-Fi Settings", NULL);
}
static void file_tap_cb(lv_obj_t *obj, void *user_data) {
  open_app(obj, "File Manager", NULL);
}
static void settings_tap_cb(lv_obj_t *obj, void *user_data) {
  open_app(obj, "Settings", NULL);
}
static void music_tap_cb(lv_obj_t *obj, void *user_data) {
  open_app(obj, "Music", NULL);
}

static void create_home(void) {
  home_view = lv_obj_create(app_container);
  lv_obj_remove_style_all(home_view);
  lv_obj_set_size(home_view, 320, 426);
  lv_obj_set_style_bg_color(home_view, lv_color_hex(0x121212), 0);
  lv_obj_set_style_bg_opa(home_view, LV_OPA_COVER, 0);

  lv_obj_t *grid = lv_obj_create(home_view);
  lv_obj_remove_style_all(grid);
  lv_obj_set_size(grid, 304, 380);
  lv_obj_set_flex_flow(grid, LV_FLEX_FLOW_ROW_WRAP);
  lv_obj_set_flex_align(grid, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_START,
                        LV_FLEX_ALIGN_START);
  lv_obj_set_style_pad_gap(grid, 8, 0);

  lv_obj_t *btn;
  btn = add_grid_app_item(grid, "Wi-Fi", LV_SYMBOL_WIFI);
  wm_on_tap(btn, wifi_tap_cb, NULL);
  wm_on_hold(btn, app_hold_cb, (void *)"Wi-Fi");

  btn = add_grid_app_item(grid, "Files", LV_SYMBOL_DIRECTORY);
  wm_on_tap(btn, file_tap_cb, NULL);
  wm_on_hold(btn, app_hold_cb, (void *)"Files");

  btn = add_grid_app_item(grid, "Settings", LV_SYMBOL_SETTINGS);
  wm_on_tap(btn, settings_tap_cb, NULL);
  wm_on_hold(btn, app_hold_cb, (void *)"Settings");

  btn = add_grid_app_item(grid, "Music", LV_SYMBOL_AUDIO);
  wm_on_tap(btn, music_tap_cb, NULL);
  wm_on_hold(btn, app_hold_cb, (void *)"Music");
}

static void create_status_bar(lv_obj_t *parent) {
  status_bar = lv_obj_create(parent);
  lv_obj_remove_style_all(status_bar);
  lv_obj_set_size(status_bar, 320, 24);
  lv_obj_set_align(status_bar, LV_ALIGN_TOP_MID);
  lv_obj_set_style_bg_color(status_bar, lv_color_hex(0x000000), 0);
  lv_obj_set_style_bg_opa(status_bar, LV_OPA_80, 0);

  lv_obj_t *title = lv_label_create(status_bar);
  lv_label_set_text(title, "VANTAGE OS");
  lv_obj_set_style_text_color(title, lv_color_hex(0xFFFFFF), 0);
  lv_obj_align(title, LV_ALIGN_LEFT_MID, 8, 0);
}

static void create_navigation_bar(lv_obj_t *parent) {
  nav_bar = lv_obj_create(parent);
  lv_obj_remove_style_all(nav_bar);
  lv_obj_set_size(nav_bar, 320, 30);
  lv_obj_align(nav_bar, LV_ALIGN_BOTTOM_MID, 0, 0);
  lv_obj_set_style_bg_color(nav_bar, lv_color_hex(0x000000), 0);
  lv_obj_set_style_bg_opa(nav_bar, LV_OPA_COVER, 0);

  lv_obj_set_flex_flow(nav_bar, LV_FLEX_FLOW_ROW);
  lv_obj_set_flex_align(nav_bar, LV_FLEX_ALIGN_SPACE_EVENLY,
                        LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);

  lv_obj_t *btn_back = lv_btn_create(nav_bar);
  lv_obj_set_size(btn_back, 60, 26);
  lv_obj_set_style_bg_opa(btn_back, LV_OPA_TRANSP, 0);
  lv_obj_t *lbl_back = lv_label_create(btn_back);
  lv_label_set_text(lbl_back, "Back");
  lv_obj_center(lbl_back);
  wm_on_tap(btn_back, app_back_cb, NULL);

  lv_obj_t *btn_home = lv_btn_create(nav_bar);
  lv_obj_set_size(btn_home, 60, 26);
  lv_obj_set_style_bg_opa(btn_home, LV_OPA_TRANSP, 0);
  lv_obj_t *lbl_home = lv_label_create(btn_home);
  lv_label_set_text(lbl_home, "Home");
  lv_obj_center(lbl_home);
  wm_on_tap(btn_home, app_home_cb, NULL);

  lv_obj_t *btn_recents = lv_btn_create(nav_bar);
  lv_obj_set_size(btn_recents, 60, 26);
  lv_obj_set_style_bg_opa(btn_recents, LV_OPA_TRANSP, 0);
  lv_obj_t *lbl_recents = lv_label_create(btn_recents);
  lv_label_set_text(lbl_recents, "Recents");
  lv_obj_center(lbl_recents);
  wm_on_tap(btn_recents, app_recents_cb, NULL);
}

void wm_init(void) {
  main_screen = lv_obj_create(NULL);
  lv_obj_set_style_bg_color(main_screen, lv_color_hex(0x0a0a0a), 0);

  app_container = lv_obj_create(main_screen);
  lv_obj_remove_style_all(app_container);
  lv_obj_set_size(app_container, 320, 426);
  lv_obj_align(app_container, LV_ALIGN_TOP_MID, 0, 24);

  create_status_bar(main_screen);
  create_navigation_bar(main_screen);
  create_home();

  lv_scr_load(main_screen);
}
