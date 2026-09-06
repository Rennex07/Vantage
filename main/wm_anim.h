#pragma once

#include <lvgl.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * @brief Animate a source object (e.g. an app icon) expanding/fading into a
 *        target object (e.g. the new app window).
 *
 * The target object is faded in from transparent to fully opaque while a
 * "ghost" copy of the source expands from the source's current position to
 * the target's current position.  The ghost is deleted when the animation
 * completes.
 *
 * @param src    The source object to zoom from.  Must be on the active screen.
 * @param target The target object to reveal (e.g. the new app root_view).
 */
void wm_animate_launch(lv_obj_t *src, lv_obj_t *target);

#ifdef __cplusplus
}
#endif
