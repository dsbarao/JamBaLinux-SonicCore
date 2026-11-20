/*---------------------------------------------------------*\
| JamBaLinuxPlugin.h                                        |
|                                                           |
|   OpenRGB plugin that exposes the JBL Quantum 810         |
|   Wireless lighting through JamBaLinux SonicCore.         |
|                                                           |
|   The plugin source is MIT licensed. It is compiled       |
|   against OpenRGB headers (GPL-2.0-or-later), so the      |
|   built plugin binary is distributed under the GPL.       |
|                                                           |
|   SPDX-License-Identifier: MIT                            |
\*---------------------------------------------------------*/

#pragma once

#include <QObject>
#include <memory>

#include "OpenRGBPluginInterface.h"
#include "SoniccoreClient.h"

class QLabel;

class JamBaLinuxPlugin : public QObject, public OpenRGBPluginInterface
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID OpenRGBPluginInterface_IID FILE "JamBaLinuxPlugin.json")
    Q_INTERFACES(OpenRGBPluginInterface)

public:
    ~JamBaLinuxPlugin() override;

    OpenRGBPluginInfo       GetPluginInfo() override;
    unsigned int            GetPluginAPIVersion() override;

    void                    Load(OpenRGBPluginAPIInterface* plugin_api_ptr) override;
    QWidget*                GetWidget() override;
    QMenu*                  GetTrayMenu() override;
    void                    Unload() override;
    void                    OnProfileAboutToLoad() override;
    void                    OnProfileLoad(nlohmann::json profile_data) override;
    nlohmann::json          OnProfileSave() override;
    unsigned char*          OnSDKCommand(unsigned int pkt_id, unsigned char* pkt_data, unsigned int* pkt_size) override;

    void                    ProfileManagerUpdated(unsigned int update_reason) override;
    void                    ResourceManagerUpdated(unsigned int update_reason) override;
    void                    SettingsManagerUpdated(unsigned int update_reason) override;

    /* Called by the virtual controller when OpenRGB changes the device. */
    void                    DeviceChanged();

private:
    void                    Log(unsigned int level, const std::string& message);

    OpenRGBPluginAPIInterface*                  api         = nullptr;
    RGBControllerInterface*                     controller  = nullptr;
    std::unique_ptr<jambalinux::LightingWriter> writer;
    std::string                                 soniccore;
    std::string                                 status_text;
};
