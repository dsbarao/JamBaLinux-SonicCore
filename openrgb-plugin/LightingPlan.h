/*---------------------------------------------------------*\
| LightingPlan.h                                            |
|                                                           |
|   Pure translation from an OpenRGB lighting state to the  |
|   `soniccore set` commands that reproduce it. It has no   |
|   Qt or OpenRGB dependency so it can be unit tested.      |
|                                                           |
|   The plugin never talks to the headset directly: every  |
|   write goes through the soniccore CLI, which owns the    |
|   allowlisted HID reports, the lighting lock and the      |
|   status cache.                                           |
|                                                           |
|   SPDX-License-Identifier: MIT                            |
\*---------------------------------------------------------*/

#pragma once

#include <array>
#include <cstdint>
#include <cstdio>
#include <optional>
#include <string>
#include <vector>

namespace jambalinux
{

/* Order matches the OpenRGB mode list the plugin registers. */
enum LightingMode : int
{
    MODE_DIRECT     = 0,    /* soniccore effect "solid"     */
    MODE_BREATHING  = 1,
    MODE_WAVE       = 2,
    MODE_GLITCH     = 3,
    MODE_OFF        = 4,
    MODE_COUNT      = 5,
};

constexpr unsigned int LED_COUNT    = 10;   /* logo 1-5, then ring 1-5    */
constexpr unsigned int SPEED_MAX    = 3;    /* index into SPEED_NAMES     */

inline const char* const EFFECT_NAMES[] = { "solid", "breathing", "wave", "glitch" };

/* Slowest to fastest, as the soniccore CLI names them. */
inline const char* const SPEED_NAMES[]  = { "0.5", "1.0", "1.5", "2.0" };

struct LightingState
{
    int                                 mode    = MODE_DIRECT;
    unsigned int                        speed   = 0;
    std::array<uint32_t, LED_COUNT>     colors  = {};   /* OpenRGB RGBColor: 0x00BBGGRR */

    bool operator==(const LightingState& other) const
    {
        return mode == other.mode && speed == other.speed && colors == other.colors;
    }
};

inline std::string ColorHex(uint32_t rgb_color)
{
    char buffer[8];
    std::snprintf(buffer, sizeof(buffer), "#%02x%02x%02x",
                  rgb_color & 0xFF, (rgb_color >> 8) & 0xFF, (rgb_color >> 16) & 0xFF);
    return buffer;
}

/* Parses "#rrggbb" into an OpenRGB RGBColor. */
inline std::optional<uint32_t> ParseColorHex(const std::string& value)
{
    if(value.size() != 7 || value[0] != '#')
    {
        return std::nullopt;
    }
    uint32_t channels[3];
    for(int channel = 0; channel < 3; channel++)
    {
        uint32_t byte = 0;
        for(int digit = 0; digit < 2; digit++)
        {
            char character = value[1 + channel * 2 + digit];
            uint32_t nibble;
            if(character >= '0' && character <= '9')      nibble = character - '0';
            else if(character >= 'a' && character <= 'f') nibble = character - 'a' + 10;
            else if(character >= 'A' && character <= 'F') nibble = character - 'A' + 10;
            else return std::nullopt;
            byte = byte * 16 + nibble;
        }
        channels[channel] = byte;
    }
    return (channels[2] << 16) | (channels[1] << 8) | channels[0];
}

inline std::string LedsArgument(const LightingState& state)
{
    std::string joined;
    for(unsigned int index = 0; index < LED_COUNT; index++)
    {
        if(index > 0)
        {
            joined += ",";
        }
        joined += ColorHex(state.colors[index]);
    }
    return joined;
}

typedef std::vector<std::string> Command;

/*---------------------------------------------------------*\
| The commands that move the headset from `applied` (the   |
| last state known to be on the device, or nothing) to     |
| `desired`. Each lighting-profile command rewrites both   |
| zones and switches lighting on, so the minimum set is    |
| sent and nothing is sent when the states already match.  |
\*---------------------------------------------------------*/
inline std::vector<Command> PlanCommands(const std::optional<LightingState>& applied,
                                         const LightingState& desired)
{
    std::vector<Command> commands;

    if(desired.mode < 0 || desired.mode >= MODE_COUNT)
    {
        return commands;
    }

    if(desired.mode == MODE_OFF)
    {
        if(!applied || applied->mode != MODE_OFF)
        {
            commands.push_back({ "set", "lighting", "off" });
        }
        return commands;
    }

    const bool          mode_changed    = !applied || applied->mode != desired.mode;
    const unsigned int  speed           = desired.speed > SPEED_MAX ? SPEED_MAX : desired.speed;

    /*-----------------------------------------------------*\
    | Colors first: `leds` replaces every color, so it works |
    | even when the cached profile is unknown, whereas the   |
    | effect and speed commands need known cached colors.    |
    \*-----------------------------------------------------*/
    if(mode_changed || applied->colors != desired.colors)
    {
        commands.push_back({ "set", "leds", LedsArgument(desired) });
    }

    if(mode_changed)
    {
        commands.push_back({ "set", "effect", EFFECT_NAMES[desired.mode] });
    }

    /* "solid" ignores the speed; animated effects need it. */
    if(desired.mode != MODE_DIRECT && (mode_changed || applied->speed != speed))
    {
        commands.push_back({ "set", "speed", SPEED_NAMES[speed] });
    }

    return commands;
}

}
