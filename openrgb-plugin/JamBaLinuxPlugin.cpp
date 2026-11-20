/*---------------------------------------------------------*\
| JamBaLinuxPlugin.cpp                                      |
|                                                           |
|   SPDX-License-Identifier: MIT                            |
\*---------------------------------------------------------*/

#include "JamBaLinuxPlugin.h"

#include <QLabel>
#include <QString>

using namespace jambalinux;

/* OpenRGB LogManager levels (LogManager.h). */
static constexpr unsigned int LOG_LEVEL_ERROR   = 1;
static constexpr unsigned int LOG_LEVEL_WARNING = 2;
static constexpr unsigned int LOG_LEVEL_INFO    = 3;

static const char* const PLUGIN_NAME = "JamBaLinux SonicCore";

/*---------------------------------------------------------*\
| Virtual controller callbacks. OpenRGB releases the       |
| controller lock before calling them, and every change    |
| (LEDs, zone, single LED, mode) is handled the same way:  |
| read the whole state and hand it to the writer.          |
\*---------------------------------------------------------*/
static void OnUpdate(void* object)
{
    static_cast<JamBaLinuxPlugin*>(object)->DeviceChanged();
}

static void OnUpdateIndexed(void* object, int)
{
    static_cast<JamBaLinuxPlugin*>(object)->DeviceChanged();
}

static mode MakeMode(const char* name, int value, bool animated)
{
    mode new_mode;
    new_mode.name       = name;
    new_mode.value      = value;
    if(value == MODE_OFF)
    {
        new_mode.flags      = 0;
        new_mode.color_mode = MODE_COLORS_NONE;
        return new_mode;
    }
    new_mode.flags      = MODE_FLAG_HAS_PER_LED_COLOR;
    new_mode.color_mode = MODE_COLORS_PER_LED;
    if(animated)
    {
        new_mode.flags     |= MODE_FLAG_HAS_SPEED;
        new_mode.speed_min  = 0;
        new_mode.speed_max  = SPEED_MAX;
        new_mode.speed      = 1;
    }
    return new_mode;
}

static zone MakeZone(const char* name)
{
    zone new_zone;
    new_zone.name       = name;
    new_zone.type       = ZONE_TYPE_LINEAR;
    new_zone.leds_count = 5;
    new_zone.leds_min   = 5;
    new_zone.leds_max   = 5;
    return new_zone;
}

JamBaLinuxPlugin::~JamBaLinuxPlugin()
{
    Unload();
}

OpenRGBPluginInfo JamBaLinuxPlugin::GetPluginInfo()
{
    OpenRGBPluginInfo info;
    info.Name            = PLUGIN_NAME;
    info.Description     = "JBL Quantum 810 Wireless lighting through the JamBaLinux SonicCore CLI";
    info.Version         = "0.1.0";
    info.Commit          = "";
    info.URL             = "https://github.com/dsbarao/JamBaLinux-SonicCore";
    info.Location        = OPENRGB_PLUGIN_LOCATION_INFORMATION;
    info.Label           = "JamBaLinux";
    info.ProtocolVersion = 0;
    return info;
}

unsigned int JamBaLinuxPlugin::GetPluginAPIVersion()
{
    return OPENRGB_PLUGIN_API_VERSION;
}

void JamBaLinuxPlugin::Log(unsigned int level, const std::string& message)
{
    if(api)
    {
        api->LogEntry(__FILE__, __LINE__, level, "[%s] %s", PLUGIN_NAME, message.c_str());
    }
}

void JamBaLinuxPlugin::Load(OpenRGBPluginAPIInterface* plugin_api_ptr)
{
    api       = plugin_api_ptr;
    soniccore = ResolveSoniccore();
    if(soniccore.empty())
    {
        status_text = "soniccore não foi encontrado em $CARGO_HOME/bin, ~/.cargo/bin ou no PATH. "
                      "Instale o JamBaLinux SonicCore com tools/install-user.sh.";
        Log(LOG_LEVEL_ERROR, "soniccore not found; the headset is not registered");
        return;
    }

    /*-----------------------------------------------------*\
    | Start from what the headset last reported, so the     |
    | first OpenRGB update only sends what actually changed |
    | and the device page shows the real colors.            |
    \*-----------------------------------------------------*/
    std::optional<LightingState> cached = ReadCachedLighting(soniccore);
    LightingState initial = cached.value_or(LightingState());

    RGBController_Setup setup{};
    setup.name          = "JBL Quantum 810 Wireless";
    setup.vendor        = "JBL";
    setup.description   = "Headset lighting through JamBaLinux SonicCore";
    setup.location      = "soniccore: " + soniccore;
    setup.version       = "";
    setup.serial        = "";
    setup.type          = DEVICE_TYPE_HEADSET;
    setup.flags         = CONTROLLER_FLAG_VIRTUAL;
    setup.object_ptr    = this;

    setup.modes.push_back(MakeMode("Direct",    MODE_DIRECT,    false));
    setup.modes.push_back(MakeMode("Breathing", MODE_BREATHING, true));
    setup.modes.push_back(MakeMode("Wave",      MODE_WAVE,      true));
    setup.modes.push_back(MakeMode("Glitch",    MODE_GLITCH,    true));
    setup.modes.push_back(MakeMode("Off",       MODE_OFF,       false));
    setup.active_mode   = initial.mode;
    if(initial.mode != MODE_DIRECT && initial.mode != MODE_OFF)
    {
        setup.modes[initial.mode].speed = initial.speed;
    }

    setup.zones.push_back(MakeZone("Logo"));
    setup.zones.push_back(MakeZone("Ring"));
    for(unsigned int index = 0; index < LED_COUNT; index++)
    {
        led new_led;
        new_led.name  = (index < 5 ? "Logo " : "Ring ") + std::to_string(index % 5 + 1);
        new_led.value = index;
        setup.leds.push_back(new_led);
    }

    setup.DeviceUpdateLEDs      = OnUpdate;
    setup.DeviceUpdateZoneLEDs  = OnUpdateIndexed;
    setup.DeviceUpdateSingleLED = OnUpdateIndexed;
    setup.DeviceUpdateMode      = OnUpdate;
    setup.DeviceUpdateZoneMode  = OnUpdateIndexed;
    setup.DeviceSaveMode        = OnUpdate;

    controller = api->CreateVirtualRGBController(&setup);
    for(unsigned int index = 0; index < LED_COUNT; index++)
    {
        controller->SetColor(index, initial.colors[index]);
    }

    writer = std::make_unique<LightingWriter>(
        soniccore,
        cached,
        [this](const std::string& message) { Log(LOG_LEVEL_WARNING, message); });

    api->RegisterVirtualRGBControllerInThread(controller);

    status_text = "JBL Quantum 810 Wireless registrado via " + soniccore + ".";
    if(!cached)
    {
        status_text += " O estado da iluminação ainda não era conhecido; a primeira "
                       "mudança feita no OpenRGB envia o perfil completo.";
    }
    Log(LOG_LEVEL_INFO, status_text);
}

void JamBaLinuxPlugin::DeviceChanged()
{
    if(!controller || !writer)
    {
        return;
    }

    LightingState state;
    int active = controller->GetActiveMode();
    state.mode  = (active >= 0 && active < MODE_COUNT) ? active : MODE_DIRECT;
    state.speed = state.mode == MODE_DIRECT || state.mode == MODE_OFF
        ? 0 : controller->GetModeSpeed(active);
    unsigned int count = controller->GetLEDCount();
    for(unsigned int index = 0; index < LED_COUNT && index < count; index++)
    {
        state.colors[index] = controller->GetColor(index);
    }
    writer->Request(state);
}

QWidget* JamBaLinuxPlugin::GetWidget()
{
    QLabel* label = new QLabel(QString::fromStdString(
        "<b>JamBaLinux SonicCore</b><br>" + status_text +
        "<br><br>Logo e anel têm 5 LEDs cada. As cores são enviadas pela CLI soniccore, "
        "no máximo uma escrita a cada 150 ms; efeitos rápidos do OpenRGB são "
        "reduzidos a essa taxa."));
    label->setWordWrap(true);
    label->setAlignment(Qt::AlignTop | Qt::AlignLeft);
    label->setTextInteractionFlags(Qt::TextSelectableByMouse);
    return label;
}

QMenu* JamBaLinuxPlugin::GetTrayMenu()
{
    return nullptr;
}

void JamBaLinuxPlugin::Unload()
{
    if(controller && api)
    {
        api->UnregisterVirtualRGBController(controller);
    }
    /* Stops the writer thread before the controller goes away. */
    writer.reset();
    if(controller && api)
    {
        api->DeleteVirtualRGBController(controller);
    }
    controller = nullptr;
}

void JamBaLinuxPlugin::OnProfileAboutToLoad()
{
}

void JamBaLinuxPlugin::OnProfileLoad(nlohmann::json)
{
}

nlohmann::json JamBaLinuxPlugin::OnProfileSave()
{
    return nlohmann::json();
}

unsigned char* JamBaLinuxPlugin::OnSDKCommand(unsigned int, unsigned char*, unsigned int*)
{
    return nullptr;
}

void JamBaLinuxPlugin::ProfileManagerUpdated(unsigned int)
{
}

void JamBaLinuxPlugin::ResourceManagerUpdated(unsigned int)
{
}

void JamBaLinuxPlugin::SettingsManagerUpdated(unsigned int)
{
}
