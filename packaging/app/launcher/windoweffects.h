/*---------------------------------------------------------*\
| windoweffects.h                                           |
|                                                           |
|   Real frosted glass for the "Vidro" background: the      |
|   desktop compositor blurs what is behind the window.     |
|   KDE's KWindowSystem is loaded at run time when it is    |
|   there, so the app needs nothing extra to build and on   |
|   other desktops nothing happens (the background keeps    |
|   its own blurred wallpaper).                             |
\*---------------------------------------------------------*/

#pragma once

class QWindow;

namespace WindowEffects
{
/* Whether the compositor can blur behind windows (KWin can). */
bool blurAvailable();

/* Asks the compositor to blur everything behind `window`, or to stop.
   Returns true when the blur is on. */
bool setBlurBehind(QWindow* window, bool enabled);
}
