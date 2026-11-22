#include "windoweffects.h"

#include <QRegion>
#include <QWindow>

#include <dlfcn.h>

namespace
{
/* KWindowEffects (KF6 KWindowSystem), by its exported C++ names:
   bool isEffectAvailable(KWindowEffects::Effect) and
   void enableBlurBehind(QWindow*, bool, const QRegion&). */
using IsEffectAvailable = bool (*)(int);
using EnableBlurBehind  = void (*)(QWindow*, bool, const QRegion&);

constexpr int BLUR_BEHIND = 7;   /* KWindowEffects::BlurBehind */

struct KWindowEffects
{
    IsEffectAvailable isEffectAvailable = nullptr;
    EnableBlurBehind  enableBlurBehind  = nullptr;
};

const KWindowEffects& library()
{
    static const KWindowEffects functions = []() {
        KWindowEffects found;
        /* Missing where KDE's frameworks are not installed: then the blur
           is simply not offered. */
        void* handle = dlopen("libKF6WindowSystem.so.6", RTLD_LAZY | RTLD_LOCAL);
        if(handle == nullptr)
        {
            return found;
        }
        found.isEffectAvailable = reinterpret_cast<IsEffectAvailable>(
            dlsym(handle, "_ZN14KWindowEffects17isEffectAvailableENS_6EffectE"));
        found.enableBlurBehind = reinterpret_cast<EnableBlurBehind>(
            dlsym(handle, "_ZN14KWindowEffects16enableBlurBehindEP7QWindowbRK7QRegion"));
        return found;
    }();
    return functions;
}
}

bool WindowEffects::blurAvailable()
{
    const KWindowEffects& functions = library();
    return functions.isEffectAvailable != nullptr && functions.enableBlurBehind != nullptr
        && functions.isEffectAvailable(BLUR_BEHIND);
}

bool WindowEffects::setBlurBehind(QWindow* window, bool enabled)
{
    if(window == nullptr || !blurAvailable())
    {
        return false;
    }
    /* An empty region is the whole window. */
    library().enableBlurBehind(window, enabled, QRegion());
    return enabled;
}
