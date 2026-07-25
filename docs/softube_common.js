// WARNING - DO NOT EDIT.
// This file is overwritten every time Softube On-Screen Display starts.
// Cubase MIDI Remote script

// get the api's entry point
var midiremote_api = require('midiremote_api_v1')

var g_remote = {}

// magic value indicating json-over-sysex
var g_sysexMagicString = "stc1"
var g_sysexMagic = []

for (var i = 0; i < g_sysexMagicString.length; ++i) {
    g_sysexMagic.push(g_sysexMagicString.charCodeAt(i))
}

const SYSEX_START = 0xF0
const SYSEX_STOP = 0xF7
const SYSEX_MANUFACTURER = 0x7D

function init(deviceName, graphicalRepresentationFunc) {
    g_remote.deviceDriver = midiremote_api.makeDeviceDriver('Softube', deviceName, 'Softube')
    g_remote.appName = midiremote_api.mDefaults.getAppName()

    g_remote.defaultPage = g_remote.deviceDriver.mMapping.makePage('Default')

    g_remote.midi = {}
    g_remote.midi.input = g_remote.deviceDriver.mPorts.makeMidiInput()
    g_remote.midi.output = g_remote.deviceDriver.mPorts.makeMidiOutput()

    g_remote.deviceDriver.makeDetectionUnit().detectPortPair(g_remote.midi.input, g_remote.midi.output)
        .expectInputNameStartsWith(deviceName)
        .expectOutputNameStartsWith(deviceName)
        .expectSysexIdentityResponse("7D", "0401", "0000")

    if (!g_remote.defaultPage.mHostAccess.makeDirectAccess) {
        console.log("The Softube Console 1 integration needs a newer version of " + g_remote.appName + ".")
        errorMessageUI(g_remote.deviceDriver, g_remote.defaultPage)
        return
    }

    graphicalRepresentationFunc(g_remote.deviceDriver, g_remote.defaultPage)

    g_remote.midi.input.mOnSysex = onSysexCallback

    g_remote.mixConsole = g_remote.defaultPage.mHostAccess.makeDirectAccess(g_remote.defaultPage.mHostAccess.mMixConsole)

    g_remote.deviceDriver.mOnActivate = deviceDriverOnActivateCallback
    g_remote.deviceDriver.mOnDeactivate = deviceDriverOnDeactivateCallback

    g_remote.defaultPage.mOnActivate = pageOnActivateCallback
    g_remote.defaultPage.mOnDeactivate = pageOnDeactivateCallback

    g_remote.mixConsole.mOnObjectChange = objectChangeCallback
    g_remote.mixConsole.mOnObjectWillBeRemoved = removeObject
    g_remote.mixConsole.mOnParameterChange = parameterChangeCallback

    g_remote.defaultPage.mOnIdle = pageIdleCallback
}

function errorMessageUI(deviceDriver, midiRemotePage)
{
    errorLabel = deviceDriver.mSurface.makeLabelField(0, 0, 100, 20)
    midiRemotePage.setLabelFieldText(errorLabel, "Disabled")

    errorLabel = deviceDriver.mSurface.makeLabelField(0, 20, 100, 20)
    midiRemotePage.setLabelFieldText(errorLabel, "Update " + g_remote.appName)
}

var NUMBER_OF_SENDS = 6

const LOG_JSON = false
const ENABLE_VU_METERS = true

var g_cachedTrackInfo = {}
var g_childToParentMap = {}
var g_parentToPanMap = {}
var g_parentToSendsMap = {}
var g_parentToInsertSlotMap = {}
var g_parentToStripSlotMap = {}
var g_parentToFilterMap = {}
var g_filterTagToParamMap = {}
var g_parentToEQMap = {}
var g_eqTagToParamMap = {}
var g_parentToCompMap = {}
var g_compTagToParamMap = {}
var g_compParamToTagMap = {}
var g_currentCompMaps = {}
var g_cachedParameter = {}

var g_enabledMetersObjectIds = {}

var g_enabled = false

// compressor parameter maps
// tube
var g_compTubeMap = {}
g_compTubeMap["Bypass"] = "compOn"
g_compTubeMap["Input Gain"] = "compComp"
g_compTubeMap["High Ratio"] = "compRatio"
g_compTubeMap["Attack Time"] = "compAttack"
g_compTubeMap["Release Time"] = "compRelease"
g_compTubeMap["Output Gain"] = "compMakeup"
g_compTubeMap["Auto Release"] = "compKnee"
g_compTubeMap["Drive"] = "compAttackShift"
g_compTubeMap["Mix"] = "compWetdry"
// standard
var g_compStandardMap = {}
g_compStandardMap["Bypass"] = "compOn"
g_compStandardMap["Threshold"] = "compComp"
g_compStandardMap["Ratio"] = "compRatio"
g_compStandardMap["Attack"] = "compAttack"
g_compStandardMap["Release"] = "compRelease"
g_compStandardMap["MakeUp"] = "compMakeup"
g_compStandardMap["Auto Release"] = "compKnee"
g_compStandardMap["Hold"] = "compAttackShift"
g_compStandardMap["DryMix"] = "compWetdry"
// vintage
var g_compVintageMap = {}
g_compVintageMap["Bypass"] = "compOn"
g_compVintageMap["Input Gain"] = "compComp"
g_compVintageMap["Ratio"] = "compRatio"
g_compVintageMap["Attack Time"] = "compAttack"
g_compVintageMap["Release Time"] = "compRelease"
g_compVintageMap["Output Gain"] = "compMakeup"
g_compVintageMap["Auto Release"] = "compKnee"
g_compVintageMap["Attack Mode"] = "compAttackShift"
g_compVintageMap["Mix"] = "compWetdry"

function sendJson(activeDevice, obj, disobey_enable) {
    var data = []
    data.push(SYSEX_START)
    data.push(SYSEX_MANUFACTURER)

    data = data.concat(g_sysexMagic)

    function fixNegativeInf (key, value) {
        // -Infinity is explicitly not support by JSON, but it is specified to work with the protobuf JSON parser in OSD so use a replacer to get it back in
        // Without this, the -Infinity property will be "null" instead of "-Infinity"
        if (typeof value === "number" && value === -Infinity) {
            return '-Infinity'
        }
        return value
    }

    var obj_str = JSON.stringify(obj, fixNegativeInf)

    for (var i = 0; i < obj_str.length; ++i) {
        c = obj_str.codePointAt(i)
        if (c >= 0x20 && c < 0x7F) {
            data.push(c)
        }
        else {
            // The output cannot have bit 7 set in any byte, use JSON standard code point escapes instead

            var escapeStr = "\\\\u"

            if (c <= 0xFF) {
                escapeStr += "00"
            }

            escapeStr += c.toString(16)
            for (var escapeIndex = 0; escapeIndex < escapeStr.length; escapeIndex++) {
                data.push(escapeStr.codePointAt(escapeIndex))
            }
        }
    }

    data.push(SYSEX_STOP)

    if (g_enabled || disobey_enable == true) {
        if (LOG_JSON) {
            console.log("Sending JSON (total length: " + data.length + "): " + obj_str)
            console.log("Sending JSON (total length: " + data.length + "): " + data.toString("hex"))
        }

        g_remote.midi.output.sendMidi(activeDevice, data)
    }
}

function createOSDMessageTrackId(trackId) {
    var message = {}
    message['trackId'] = trackId
    message['isActive'] = false
    return message
}

function createOSDMessageTrackNumber(trackNumber) {
    var message = {}
    message['track'] = trackNumber
    message['isActive'] = false
    return message
}

function handleTrackPropertyChange(activeDevice, storedTrackInfo, valueName, value, extras) {
    if (valueName == 'isActive') {
        if (value == true && storedTrackInfo['isActive'] == false) {
            // This track went active and we should send all saved properties for it
            //console.log("Activating track: " + storedTrackInfo['track'].toString())
            storedTrackInfo['isActive'] = true
            if (g_enabled) {
                sendJson(activeDevice, storedTrackInfo)
            }
            return true
        }
        return false
    }

    // Send if property is new or changed
    var send = false
    if (((storedTrackInfo[valueName] === undefined) || (typeof value === 'object' && storedTrackInfo[valueName] != value) || (storedTrackInfo[valueName].toString() != value.toString())) && (storedTrackInfo['isActive'] == true)) {
        send = true
    }

    storedTrackInfo[valueName] = value

    if (send) {
        var obj = createOSDMessageTrackId(storedTrackInfo['trackId'])
        delete obj['isActive']
        obj[valueName] = value
        if (extras) {
            Object.keys(extras).forEach(function (key) {
                obj[key] = extras[key]
            })
        }

        sendJson(activeDevice, obj)
    }
    return send
}

function colorToNumber(r, g, b) {
    var newColor = 0
    newColor |= ((r * 0xFF) << 0)
    newColor |= ((g * 0xFF) << 8)
    newColor |= ((b * 0xFF) << 16)
    return newColor
}

function TrackId2ObjectId(trackId) {
    for (var objectID in g_cachedTrackInfo) {
        var cachedTrackIdStr = g_cachedTrackInfo[objectID]['trackId'].toString()
        var trackIdStr = trackId.toString()
        if (cachedTrackIdStr === trackIdStr) {
            return objectID
        }
    }
}

/**
 * Callback for received Sysex messages (sent from Softube Softube On-Screen Display to Cubase/Nuendo)
 */
function onSysexCallback(activeDevice, msg) {
    // sysex start + id + magic string + at least two bytes payload: {} + stop byte
    var minMsgLength = 2 +  g_sysexMagic.length + 2 + 1

    // check message length
    if (msg.length < minMsgLength) {
        return
    }

    // Check sysex start, id, sysex end bytes
    if (msg[0] != SYSEX_START || msg[1] != SYSEX_MANUFACTURER || msg[msg.length - 1] != SYSEX_STOP) {
        return
    }

    // Check magic string
    for (var i = 0; i < g_sysexMagic.length; ++i) {
        if (msg[i + 2] != g_sysexMagic[i]) {
            return
        }
    }

    var sliced = msg.slice(2 + g_sysexMagic.length , -1) // remove Sysex start byte, id, magic string and also the Sysex end byte
    var msg_str = ""
    for (var i = 0; i < sliced.length; i++) {
        msg_str += String.fromCharCode(sliced[i])
    }

    if (LOG_JSON) {
        console.log("Received JSON: " + msg_str)
    }

    try {
        var json_msg = JSON.parse(msg_str)

        if ("cmd" in json_msg && json_msg.cmd === 'RESET') {
            handshakeStart(activeDevice)
        }
        else if ("cmd" in json_msg && json_msg.cmd === 'ENABLE') {
            enableOSD()
        }
        else if ("cmd" in json_msg && json_msg.cmd === 'DISABLE') {
            disableOSD()
        }
        else if ('handshake' in json_msg) {
            if ('ack' in json_msg.handshake && json_msg.handshake.ack == true) {
                enableOSD()
                batchSendAllTracks(activeDevice)
                batchSendAllCachedParameters(activeDevice)
            }
        }
        else if ('activeMeters' in json_msg) {
            g_enabledMetersObjectIds = {}
            for (var i = 0; i < json_msg.activeMeters.length; i++) {
                var trackId = json_msg.activeMeters[i];
                var objectID = TrackId2ObjectId(trackId)
                if (objectID != undefined) {
                    g_enabledMetersObjectIds[objectID] = true;
                }
            }
        }

        if ("trackId" in json_msg) {
            var objectID = TrackId2ObjectId(json_msg.trackId)

            if (!objectID) {
                return
            }

            if (!g_remote.activeMapping) {
                return
            }

            var touchStateInMsg = false
            var touchStateValue = false;
            if ('touchState' in json_msg) {
                touchStateInMsg = true

                if (json_msg.touchState === 'TOUCHED') {
                    touchStateValue = true
                }
                if (json_msg.touchState === 'RELEASED') {
                    touchStateValue = false
                }

                // TODO We might need to add a touch timeout using one of the mIdle callbacks
            }

            if ('selected' in json_msg) {
                g_remote.mixConsole.setParameterProcessValue(g_remote.activeMapping, parseInt(objectID), PARAMTAG.SELECTED, +json_msg.selected)
            }
            if ('volume' in json_msg) {
                if (touchStateInMsg && touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(objectID), PARAMTAG.VOLUME, Boolean(touchStateValue))
                }
                var volumeDbStr = '-oo'
                if (json_msg.volume > -Infinity)
                {
                    volumeDbStr = json_msg.volume.toString()
                }
                g_remote.mixConsole.setParameterDisplayValue(g_remote.activeMapping, parseInt(objectID), PARAMTAG.VOLUME, volumeDbStr)

                if (touchStateInMsg && !touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(objectID), PARAMTAG.VOLUME, Boolean(touchStateValue))
                }
            }
            if ('pan' in json_msg) {

                var panObjectID = g_parentToPanMap[objectID]

                if (touchStateInMsg && touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(panObjectID), PARAMTAG.PAN, Boolean(touchStateValue))
                }

                g_remote.mixConsole.setParameterProcessValue(g_remote.activeMapping, parseInt(panObjectID), PARAMTAG.PAN, json_msg.pan)

                if (touchStateInMsg && !touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(panObjectID), PARAMTAG.PAN, Boolean(touchStateValue))
                }
            }
            if ('mute' in json_msg) {
                g_remote.mixConsole.setParameterProcessValue(g_remote.activeMapping, parseInt(objectID), PARAMTAG.MUTE, +json_msg.mute)
            }
            if ('solo' in json_msg) {
                g_remote.mixConsole.setParameterProcessValue(g_remote.activeMapping, parseInt(objectID), PARAMTAG.SOLO, +json_msg.solo)
            }

            function sendSetValue(iSend, value) {
                var sendObjectID = g_parentToSendsMap[objectID][iSend - 1]

                if (touchStateInMsg && touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(sendObjectID), PARAMTAG.SEND_LEVEL, Boolean(touchStateValue))
                }

                var sendLevelDbStr = '-oo'
                if (value > -Infinity) {
                    sendLevelDbStr = value.toString()
                }

                g_remote.mixConsole.setParameterDisplayValue(g_remote.activeMapping, parseInt(sendObjectID), PARAMTAG.SEND_LEVEL, sendLevelDbStr)

                if (touchStateInMsg && !touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, parseInt(panObjectID), PARAMTAG.PAN, Boolean(touchStateValue))
                }
            }

            if ('send1' in json_msg) {
                sendSetValue(1, json_msg.send1)
            }
            if ('send2' in json_msg) {
                sendSetValue(2, json_msg.send2)
            }
            if ('send3' in json_msg) {
                sendSetValue(3, json_msg.send3)
            }
            if ('send4' in json_msg) {
                sendSetValue(4, json_msg.send4)
            }
            if ('send5' in json_msg) {
                sendSetValue(5, json_msg.send5)
            }
            if ('send6' in json_msg) {
                sendSetValue(6, json_msg.send6)
            }

            if ('plugin' in json_msg) {
                var pluginUID = undefined

                if (json_msg.plugin === 'Console 1')
                {
                    pluginUID = '2FF966F3A2DA4112BBB38DC29B336457'
                }
                else if (json_msg.plugin === 'Flow Mixing Suite')
                {
                    pluginUID = '74D14512EBBF4BBA9F0E508E4A0EAEC6'
                }

                if (pluginUID && g_parentToInsertSlotMap[objectID])
                {
                    var slotObjectID = g_parentToInsertSlotMap[objectID]
                    g_remote.mixConsole.mPluginManager.trySetSlotPlugin(g_remote.activeMapping, slotObjectID, pluginUID, true)
                }
            }

            function setParameterValue(objectID, tag, value)
            {
                // make sure the value is not undefined
                if (!value)
                    value = 0

                // clamp
                if (value < 0.0)
                    value = 0.0
                else if (value > 1.0)
                    value = 1.0

                if (touchStateInMsg && touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, objectID, tag, Boolean(touchStateValue))
                }

                g_remote.mixConsole.setParameterProcessValue(g_remote.activeMapping, objectID, tag, value)

                if (touchStateInMsg && !touchStateValue) {
                    g_remote.mixConsole.setParameterEditLockState(g_remote.activeMapping, objectID, tag, Boolean(touchStateValue))
                }
            }

            // filter
            if ('filterPreGain' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_PRE_GAIN, json_msg.filterPreGain.value)
            }
            if ('filterPhaseInvert' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_PHASE_INVERT, json_msg.filterPhaseInvert.value)
            }
            if ('filterLcOn' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_LOW_CUT_ON, json_msg.filterLcOn.value)
            }
            if ('filterLcFreq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_LOW_CUT_FREQ, json_msg.filterLcFreq.value)
            }
            if ('filterLcSlope' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_LOW_CUT_SLOPE, json_msg.filterLcSlope.value)
            }
            if ('filterHcOn' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_HIGH_CUT_ON, json_msg.filterHcOn.value)
            }
            if ('filterHcFreq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_HIGH_CUT_FREQ, json_msg.filterHcFreq.value)
            }
            if ('filterHcSlope' in json_msg)
            {
                setParameterValue(parseInt(g_parentToFilterMap[objectID]), PARAMTAG.FILTER_HIGH_CUT_SLOPE, json_msg.filterHcSlope.value)
            }

            // EQ Band 1
            if ('eq1On' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ1_ON, json_msg.eq1On.value)
            }
            if ('eq1Type' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ1_TYPE, json_msg.eq1Type.value)
            }
            if ('eq1Gain' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ1_GAIN, json_msg.eq1Gain.value)
            }
            if ('eq1Freq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ1_FREQ, json_msg.eq1Freq.value)
            }
            if ('eq1Q' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ1_Q, json_msg.eq1Q.value)
            }

            // EQ Band 2
            if ('eq2On' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ2_ON, json_msg.eq2On.value)
            }
            if ('eq2Type' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ2_TYPE, json_msg.eq2Type.value)
            }
            if ('eq2Gain' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ2_GAIN, json_msg.eq2Gain.value)
            }
            if ('eq2Freq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ2_FREQ, json_msg.eq2Freq.value)
            }
            if ('eq2Q' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ2_Q, json_msg.eq2Q.value)
            }

            // EQ Band 3
            if ('eq3On' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ3_ON, json_msg.eq3On.value)
            }
            if ('eq3Type' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ3_TYPE, json_msg.eq3Type.value)
            }
            if ('eq3Gain' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ3_GAIN, json_msg.eq3Gain.value)
            }
            if ('eq3Freq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ3_FREQ, json_msg.eq3Freq.value)
            }
            if ('eq3Q' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ3_Q, json_msg.eq3Q.value)
            }

            // EQ Band 4
            if ('eq4On' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ4_ON, json_msg.eq4On.value)
            }
            if ('eq4Type' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ4_TYPE, json_msg.eq4Type.value)
            }
            if ('eq4Gain' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ4_GAIN, json_msg.eq4Gain.value)
            }
            if ('eq4Freq' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ4_FREQ, json_msg.eq4Freq.value)
            }
            if ('eq4Q' in json_msg)
            {
                setParameterValue(parseInt(g_parentToEQMap[objectID]), PARAMTAG.EQ4_Q, json_msg.eq4Q.value)
            }

            // comp
            if ('compOn' in json_msg)
            {
                // if no comp is loaded, load one
                if (!g_parentToCompMap[objectID] && json_msg.compOn.value == 1.0) {
                    if (g_parentToStripSlotMap[objectID].strip && g_parentToStripSlotMap[objectID].strip.comp)
                    {
                        var slotObjectID = g_parentToStripSlotMap[objectID].strip.comp
                        var pluginUID = 'E022B5972163463CBA2036708D5AF5A5' // Standard Compressor

                        g_remote.mixConsole.mPluginManager.trySetSlotPlugin(g_remote.activeMapping, slotObjectID, pluginUID, true)
                    }
                } else {
                    var compObjectId = parseInt(g_parentToCompMap[objectID])
                    setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compOn, 1.0 - json_msg.compOn.value)
                }
            }
            if ('compComp' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compComp, json_msg.compComp.value)

            }
            if ('compRatio' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compRatio, json_msg.compRatio.value)
            }
            if ('compAttack' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compAttack, json_msg.compAttack.value)
            }
            if ('compRelease' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compRelease, json_msg.compRelease.value)
            }
            if ('compMakeup' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compMakeup, json_msg.compMakeup.value)
            }
            if ('compKnee' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compKnee, json_msg.compKnee.value)
            }
            if ('compAttackShift' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compAttackShift, json_msg.compAttackShift.value)
            }
            if ('compWetdry' in json_msg)
            {
                var compObjectId = parseInt(g_parentToCompMap[objectID])
                setParameterValue(compObjectId, g_compParamToTagMap[compObjectId].compWetdry, json_msg.compWetdry.value)
            }
        }
    }
    catch (e) {
        console.log('Error parsing received JSON: ' + e.toString())
    }
}

function enableOSD() {
    // set enabled flag
    g_enabled = true;
}

function disableOSD() {
    g_enabled = false;
}

// Send all track info
function batchSendAllTracks(activeDevice) {
    var s = {
        trackBatch: []
    };

    var keys = Object.keys(g_cachedTrackInfo);
    for (var i = 0; i < keys.length; i++) {
        s.trackBatch.push(g_cachedTrackInfo[keys[i]]);

        if (s.trackBatch.length > 100) {
            sendJson(activeDevice, s);
            s.trackBatch.length = 0;
        }
    }

    if (s.trackBatch.length > 0) {
        sendJson(activeDevice, s);
    }
}

function batchSendAllCachedParameters(activeDevice) {
    for (var objectId in g_cachedParameter) {
        for (var paramTag in g_cachedParameter[objectId]) {
            var objectID = g_cachedParameter[objectId][paramTag].objectID

            parameterChangeCallback(activeDevice, g_remote.activeMapping, parseInt(objectID), parseInt(paramTag), true)
        }
    }
}

// Send all meters that have changed
function batchSendChangedMeters(activeDevice, activeMapping) {
    var s = {
        trackBatch: []
    };

    for (var objectID in g_enabledMetersObjectIds) {
        storedTrackInfo = g_cachedTrackInfo[objectID]

        if (storedTrackInfo == undefined || !storedTrackInfo.isActive) {
            continue
        }

        var meterValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, parseInt(objectID), PARAMTAG.METER);
        var peakValue = meterValue * Math.sqrt(2);

        if (validParamValue(peakValue)) {
            if (storedTrackInfo.meter === undefined || storedTrackInfo.meter[0] != peakValue)
            {
                storedTrackInfo.meter = [peakValue];
                s.trackBatch.push({ trackId: storedTrackInfo.trackId, meter: storedTrackInfo.meter })
            }
        }
    }

    if (s.trackBatch.length > 0) {
        sendJson(activeDevice, s);
    }
}

function handshakeStart(activeDevice) {
    sendJson(activeDevice, {
        handshake: {
            dawName: g_remote.appName,
            protocolVersion: [1, 1]
        }
    }, true)
}

function deviceDriverOnActivateCallback(activeDevice) {
    disableOSD()
    handshakeStart(activeDevice)
}


// Kill all tracks when deactivating
function deviceDriverOnDeactivateCallback(activeDevice) {
    //console.log('Console 1 deactivation')

    sendJson(activeDevice, {
        cmd: 'RESET'
    }, true)

    Object.keys(g_cachedTrackInfo).forEach(function (objectId) {
        removeObject(activeDevice, null, objectId, true)
    })
    disableOSD()
}

const PARAMTAG = {
    NAME: 1024,
    VOLUME: 1025,
    MUTE: 1027,
    SOLO: 1028,
    SELECTED: 4000,
    METER: 4009,
    PEAK: 4012,

    // in panner object
    PAN: 4201,

    // in send slot object
    SEND_ON: 4096,
    SEND_LEVEL: 4097,

    // Filter
    FILTER_PRE_GAIN: 4201,
    FILTER_PHASE_INVERT: 4202,
    FILTER_LOW_CUT_ON: 4203,
    FILTER_LOW_CUT_FREQ: 4204,
    FILTER_LOW_CUT_SLOPE: 4210,
    FILTER_HIGH_CUT_ON: 4205,
    FILTER_HIGH_CUT_FREQ: 4206,
    FILTER_HIGH_CUT_SLOPE: 4211,

    // EQ
    EQ1_ON: 4224,
    EQ1_TYPE: 4228,
    EQ1_GAIN: 4232,
    EQ1_FREQ: 4236,
    EQ1_Q: 4240,

    EQ2_ON: 4225,
    EQ2_TYPE: 4229,
    EQ2_GAIN: 4233,
    EQ2_FREQ: 4237,
    EQ2_Q: 4241,

    EQ3_ON: 4226,
    EQ3_TYPE: 4230,
    EQ3_GAIN: 4234,
    EQ3_FREQ: 4238,
    EQ3_Q: 4242,

    EQ4_ON: 4227,
    EQ4_TYPE: 4231,
    EQ4_GAIN: 4235,
    EQ4_FREQ: 4239,
    EQ4_Q: 4243,
}


function pageOnActivateCallback(activeDevice, activeMapping) {
    g_remote.mixConsole.activate(activeMapping)
    g_remote.activeMapping = activeMapping
}

function pageOnDeactivateCallback (activeDevice, activeMapping) {
    g_remote.mixConsole.deactivate(activeMapping)
}

function logObject(activeMapping, objectID, skipparams) {
    var title = g_remote.mixConsole.getObjectTitle(activeMapping, objectID)
    var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, objectID)
    var color = g_remote.mixConsole.getObjectColor(activeMapping, objectID)
    console.log('-  Log object start ---------------------')
    console.log('objectID: ' + objectID)
    console.log('uniqueName: ' + uniqueName)
    console.log('title: ' + title)
    //console.log('color: ' + JSON.stringify(color))

    if (!skipparams) {
        var numParams = g_remote.mixConsole.getNumberOfParameters(activeMapping, objectID)
        for (var k = 0; k < numParams; ++k) {
            var paramTag = g_remote.mixConsole.getParameterTagByIndex(activeMapping, objectID, k)
            var title = g_remote.mixConsole.getParameterTitle(activeMapping, objectID, paramTag, 16)
            var displayValue = g_remote.mixConsole.getParameterDisplayValue(activeMapping, objectID, paramTag)
            var displayUnits = g_remote.mixConsole.getParameterDisplayUnits(activeMapping, objectID, paramTag)
            var processValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
            var defaultProcessValue = g_remote.mixConsole.getParameterDefaultProcessValue(activeMapping, objectID, paramTag)
            console.log(paramTag + ':  ' + title + ' = ' + displayValue + (displayUnits ? ' ' + displayUnits : '') + ' (process value: ' + processValue.toString() + ', default process value: ' + defaultProcessValue.toString() + ')')
        }
    }

    console.log('-  Log object done ---------------------')
}

function objectID2SendSlotNumber(sendsObjectArray, sendSlotObjectId) {
    for (var i = 0; i < sendsObjectArray.length; i++) {
        if (sendsObjectArray[i] === sendSlotObjectId) {
            return i + 1;
        }
    }
    return -1
}

function forEachChild(da, mapping, parentObjectID, visit) {
    var childCount = da.getNumberOfChildObjects(mapping, parentObjectID)
    for (var i = 0; i < childCount; ++i) {
        var childID = da.getChildObjectID(mapping, parentObjectID, i)
        if (visit(childID) === false)
            return false
    }
    return true
}

function objectChangeCallback(activeDevice, activeMapping, objectID) {
    // console.log('objectChangeCallback **************************************************************')

    var baseObjectID = g_remote.mixConsole.getBaseObjectID(activeMapping)
    if (baseObjectID !== objectID)
        return

    var maxVolumeChanged = false
    var maxSendVolumeChanged = false
    var resendData = false

    var numChildren = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, objectID)
    for (var i = 0; i < numChildren; ++i) {
        var childID = g_remote.mixConsole.getChildObjectID(activeMapping, baseObjectID, i)
        var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID)
        var uniqueIDString = g_remote.mixConsole.getObjectUniqueIDString(activeMapping, childID)

        // console.log('**************************************************************')

        // console.log('Child index: ' + i.toString())

        // skip Input Channels (unique name begins with 'InputChannel')
        if (uniqueName.indexOf('InputChannel') === 0) {
            //console.log('Skipped InputChannel')
            continue
        }

        // skip Output Channels (unique name begins with 'OutputChannel')
        if (uniqueName.indexOf('OutputChannel') === 0) {
            //console.log('Skipped OutputChannel')
            continue
        }

        var trackNumber = g_remote.mixConsole.getMixerChannelIndex(activeMapping, childID)

        if (!(childID in g_cachedTrackInfo)) {
            g_cachedTrackInfo[childID] = createOSDMessageTrackNumber(trackNumber)
        }

        var colorList = g_remote.mixConsole.getObjectColor(activeMapping, childID)
        var colorNumber = colorToNumber(colorList[0], colorList[1], colorList[2])

        handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'trackId', uniqueIDString)
        handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'track', trackNumber)
        handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'color', colorNumber)

        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.NAME)
        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.VOLUME)
        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.MUTE)
        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.SOLO)
        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.SELECTED)
        parameterChangeCallback(activeDevice, activeMapping, childID, PARAMTAG.METER)

        var maxVolumeLevel = 6.02
        if (midiremote_api.mDefaults.getMaxGainFactor() == 4) {
            maxVolumeLevel = 12.04
        }

        maxVolumeChanged = handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'maxVolumeValue', maxVolumeLevel)
        maxSendVolumeChanged = handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'maxSendValue', maxVolumeLevel)

        // This is needed because the last child we loop over is not affected by the maxvolume send so they are set to false the last loop
        if(maxVolumeChanged || maxSendVolumeChanged){
            resendData = true;
        }

        // var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID)
        // var colorList = g_remote.mixConsole.getObjectColor(activeMapping, childID)
        // var numParams = g_remote.mixConsole.getNumberOfParameters(activeMapping, childID)
        // for (var k = 0; k < numParams; ++k) {
        //     var paramTag = g_remote.mixConsole.getParameterTagByIndex(activeMapping, childID, k)
        //     var title = g_remote.mixConsole.getParameterTitle(activeMapping, childID, paramTag, 16)
        //     var displayValue = g_remote.mixConsole.getParameterDisplayValue(activeMapping, childID, paramTag)
        //     var displayUnits = g_remote.mixConsole.getParameterDisplayUnits(activeMapping, childID, paramTag)
        //     console.log(paramTag + ':  ' + title + ' = ' + displayValue + (displayUnits ? ' ' + displayUnits : ''))
        // }

        // console.log('**************************************************************')

        var numChildren2 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID)
        for (var i2 = 0; i2 < numChildren2; ++i2) {
            var childID2 = g_remote.mixConsole.getChildObjectID(activeMapping, childID, i2)
            var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID2)

            // var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID2)
            // var color = g_remote.mixConsole.getObjectColor(activeMapping, childID2)
            // console.log('      -------------------------')
            // console.log('      objectID: ' + childID2)
            // console.log('      uniqueName: ' + uniqueName)
            // console.log('      title: ' + title)
            // console.log('      color: ' + JSON.stringify(color))

            // logObject(activeMapping, childID2)

            if (uniqueName === 'Inserts') {
                // find first available insert slot
                var numChildren3 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID2)
                for (var i3 = 0; i3 < numChildren3; ++i3) {
                    var childID3 = g_remote.mixConsole.getChildObjectID(activeMapping, childID2, i3)
                    // var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID3)
                    // var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID3)

                    var numChildren4 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID3)
                    if (numChildren4 == 0)
                    {
                        g_parentToInsertSlotMap[childID] = childID3
                        break;
                    }
                }
            }
            else if (uniqueName === 'InputFilter') {
                g_childToParentMap[childID2] = childID
                g_parentToFilterMap[childID] = childID2

                if (g_filterTagToParamMap[childID2] === undefined)
                    g_filterTagToParamMap[childID2] = {}

                // pre gain
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_PRE_GAIN] = "filterPreGain"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_PRE_GAIN)

                // phase invert
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_PHASE_INVERT] = "filterPhaseInvert"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_PHASE_INVERT)

                // low cut
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_LOW_CUT_ON] = "filterLcOn"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_LOW_CUT_ON)
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_LOW_CUT_FREQ] = "filterLcFreq"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_LOW_CUT_FREQ)
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_LOW_CUT_SLOPE] = "filterLcSlope"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_LOW_CUT_SLOPE)

                // high cut
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_HIGH_CUT_ON] = "filterHcOn"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_HIGH_CUT_ON)
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_HIGH_CUT_FREQ] = "filterHcFreq"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_HIGH_CUT_FREQ)
                g_filterTagToParamMap[childID2][PARAMTAG.FILTER_HIGH_CUT_SLOPE] = "filterHcSlope"
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.FILTER_HIGH_CUT_SLOPE)
            }
            else if (uniqueName === 'Strips') {
                var numChildren3 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID2)
                for (var i3 = 0; i3 < numChildren3; ++i3) {
                    var childID3 = g_remote.mixConsole.getChildObjectID(activeMapping, childID2, i3)
                    var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID3)
                    var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID3)

                    if (!g_parentToStripSlotMap[childID])
                        g_parentToStripSlotMap[childID] = {}

                    if (!g_parentToStripSlotMap[childID].strip)
                        g_parentToStripSlotMap[childID].strip = {}

                    if (i3 == 1)
                        g_parentToStripSlotMap[childID].strip.comp = childID3;

                    var numChildren4 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID3)
                    for (var i4 = 0; i4 < numChildren4; ++i4) {
                        var childID4 = g_remote.mixConsole.getChildObjectID(activeMapping, childID3, i4)
                        var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID4)
                        var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID4)

                        if (title === "EQ")
                        {
                            g_childToParentMap[childID4] = childID
                            g_parentToEQMap[childID] = childID4

                            // add missing map structure if needed
                            if (g_eqTagToParamMap[childID4] === undefined)
                                g_eqTagToParamMap[childID4] = {}

                            g_eqTagToParamMap[childID4][PARAMTAG.EQ1_ON] = "eq_1_on"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ1_ON)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ1_TYPE] = "eq_1_type"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ1_TYPE)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ1_GAIN] = "eq_1_gain"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ1_GAIN)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ1_FREQ] = "eq_1_freq"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ1_FREQ)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ1_Q] = "eq_1_q"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ1_Q)

                            g_eqTagToParamMap[childID4][PARAMTAG.EQ2_ON] = "eq_2_on"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ2_ON)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ2_TYPE] = "eq_2_type"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ2_TYPE)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ2_GAIN] = "eq_2_gain"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ2_GAIN)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ2_FREQ] = "eq_2_freq"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ2_FREQ)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ2_Q] = "eq_2_q"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ2_Q)

                            g_eqTagToParamMap[childID4][PARAMTAG.EQ3_ON] = "eq_3_on"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ3_ON)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ3_TYPE] = "eq_3_type"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ3_TYPE)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ3_GAIN] = "eq_3_gain"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ3_GAIN)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ3_FREQ] = "eq_3_freq"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ3_FREQ)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ3_Q] = "eq_3_q"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ3_Q)

                            g_eqTagToParamMap[childID4][PARAMTAG.EQ4_ON] = "eq_4_on"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ4_ON)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ4_TYPE] = "eq_4_type"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ4_TYPE)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ4_GAIN] = "eq_4_gain"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ4_GAIN)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ4_FREQ] = "eq_4_freq"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ4_FREQ)
                            g_eqTagToParamMap[childID4][PARAMTAG.EQ4_Q] = "eq_4_q"
                            parameterChangeCallback(activeDevice, activeMapping, childID4, PARAMTAG.EQ4_Q)
                        }
                        else if (title.toLowerCase().includes("compressor"))
                        {
                            g_childToParentMap[childID4] = childID
                            g_parentToCompMap[childID] = childID4

                            if (title.toLowerCase().includes("tube"))
                            {
                                g_currentCompMaps[childID4] = g_compTubeMap;
                            }
                            else if (title.toLowerCase().includes("standard"))
                            {
                                g_currentCompMaps[childID4] = g_compStandardMap
                            }
                            else if (title.toLowerCase().includes("vintage"))
                            {
                                g_currentCompMaps[childID4] = g_compVintageMap
                            }

                            var numParams = g_remote.mixConsole.getNumberOfParameters(activeMapping, childID4)
                            for (var k = 0; k < numParams; ++k) {
                                var paramTag = g_remote.mixConsole.getParameterTagByIndex(activeMapping, childID4, k)
                                var title = g_remote.mixConsole.getParameterTitle(activeMapping, childID4, paramTag, 16)

                                if (g_currentCompMaps[childID4][title])
                                {
                                    // add missing map structure if needed
                                    if (g_compTagToParamMap[childID4] === undefined)
                                        g_compTagToParamMap[childID4] = {}
                                    if (g_compParamToTagMap[childID4] === undefined)
                                        g_compParamToTagMap[childID4] = {}

                                    // tag to
                                    g_compTagToParamMap[childID4][paramTag] = g_currentCompMaps[childID4][title]
                                    g_compParamToTagMap[childID4][g_currentCompMaps[childID4][title]] = paramTag

                                    // parameter change callback
                                    parameterChangeCallback(activeDevice, activeMapping, childID4, paramTag)
                                }
                            }
                        }
                    }
                }
            }
            else if (uniqueName === 'Panner') {
                g_childToParentMap[childID2] = childID
                g_parentToPanMap[childID] = childID2
                parameterChangeCallback(activeDevice, activeMapping, childID2, PARAMTAG.PAN)
            }
            else if (uniqueName === 'Sends') {
                var numSendsSlots = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID2)

                if (!(childID in g_parentToSendsMap)) {
                    g_parentToSendsMap[childID] = []
                }

                for (var iSendSlotIndex = 0; iSendSlotIndex < Math.min(numSendsSlots, NUMBER_OF_SENDS); ++iSendSlotIndex) {
                    var sendSlotID = g_remote.mixConsole.getChildObjectID(activeMapping, childID2, iSendSlotIndex)

                    g_childToParentMap[sendSlotID] = childID
                    g_parentToSendsMap[childID].push(sendSlotID)

                    parameterChangeCallback(activeDevice, activeMapping, sendSlotID, PARAMTAG.SEND_ON)
                    parameterChangeCallback(activeDevice, activeMapping, sendSlotID, PARAMTAG.SEND_LEVEL)

                    // var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID3)
                    // var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID3)
                    // var color = g_remote.mixConsole.getObjectColor(activeMapping, childID3)
                    // console.log('         ----------------------')
                    // console.log('         objectID: ' + childID3)
                    // console.log('         uniqueName: ' + uniqueName)
                    // console.log('         title: ' + title)
                    // console.log('         color: ' + JSON.stringify(color))

                    // logObject(activeMapping, childID3)
                }
            }


            // if (uniqueName === 'Sends') {
            //     var numChildren3 = g_remote.mixConsole.getNumberOfChildObjects(activeMapping, childID2)
            //     for (var i3 = 0; i3 < numChildren3; ++i3) {
            //         var childID3 = g_remote.mixConsole.getChildObjectID(activeMapping, childID2, i3)
            //         var title = g_remote.mixConsole.getObjectTitle(activeMapping, childID3)
            //         var uniqueName = g_remote.mixConsole.getObjectUniqueName(activeMapping, childID3)
            //         var color = g_remote.mixConsole.getObjectColor(activeMapping, childID3)
            //         console.log('         ----------------------')
            //         console.log('         objectID: ' + childID3)
            //         console.log('         uniqueName: ' + uniqueName)
            //         console.log('         title: ' + title)
            //         console.log('         color: ' + JSON.stringify(color))

            //         logObject(activeMapping, childID3)
            //     }
            // }

            // console.log('         ----------------------')
        }

        var isVisible = g_remote.mixConsole.isMixerChannelVisible(activeMapping, childID)
        handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[childID], 'isActive', Boolean(isVisible))
    }

    if(resendData)
    {
        batchSendAllTracks(activeDevice)
        batchSendAllCachedParameters(activeDevice)
    }

    //console.log(JSON.stringify(g_cachedTrackInfo))
}

function removeObject(activeDevice, activeMapping, objectID, skipMessage) {
    var parentID = g_childToParentMap[objectID]

    // deletion helper function
    function TryRemoveFromObject(object, objectID)
    {
        try {
            delete object[objectID]
        }
        catch (e) {
            // do nothing
        }
    }

    // try to clear comp parameters if they exist
    if (g_currentCompMaps[objectID]) {
        keys = Object.keys(g_currentCompMaps[objectID])
        for (var iKey = 0; iKey < keys.length; iKey++) {
            var name = g_currentCompMaps[objectID][keys[iKey]]
            var paramTag = g_compParamToTagMap[objectID][name]

            // we send a blank name for each parameter in map
            if (g_cachedTrackInfo[parentID]) {
                handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[parentID], name, {
                    name: ""
                })
            }
        }

        TryRemoveFromObject(g_currentCompMaps, objectID)
    }

    // remove cached track info
    if (objectID in g_cachedTrackInfo) {
        if (skipMessage !== true) {
            var obj = createOSDMessageTrackId(g_cachedTrackInfo[objectID]['trackId'])
            sendJson(activeDevice, obj)
        }
        delete g_cachedTrackInfo[objectID]
    }

    // Attempt some further cleanup:
    TryRemoveFromObject(g_childToParentMap, objectID)
    TryRemoveFromObject(g_parentToPanMap, objectID)
    TryRemoveFromObject(g_parentToSendsMap, objectID)
    TryRemoveFromObject(g_parentToInsertSlotMap, objectID)
    TryRemoveFromObject(g_parentToStripSlotMap, objectID)
    TryRemoveFromObject(g_parentToFilterMap, parentID)
    TryRemoveFromObject(g_parentToEQMap, objectID)
    TryRemoveFromObject(g_parentToCompMap, objectID)
    TryRemoveFromObject(g_compTagToParamMap, objectID)
    TryRemoveFromObject(g_compParamToTagMap, objectID)
    TryRemoveFromObject(g_cachedParameter, objectID)
}

function validParamValue(value) {
    return value >= 0.0 && value <= 1.0
}

function parameterChangeCallback(activeDevice, activeMapping, objectID, paramTag, includeInfo) {
    if (objectID in g_cachedTrackInfo || objectID in g_childToParentMap) {

        // parameter value change function
        function handleParameterValueChange(objectID, paramTag, name, includeInfo) {
            var parentID = g_childToParentMap[objectID]
            var displayName = undefined
            var quantisation = undefined
            var value = undefined
            var displayValue = undefined
            var defaultValue = undefined

            // make sure we have the correct cache structure
            if (!g_cachedParameter[objectID])
                g_cachedParameter[objectID] = {}

            if (!g_cachedParameter[objectID][paramTag])
                g_cachedParameter[objectID][paramTag] = {}

            // name
            var newTitle = g_remote.mixConsole.getParameterTitle(activeMapping, objectID, paramTag, 128)

            if (g_cachedParameter[objectID][paramTag].title != newTitle || includeInfo) // we only want to send the name when i changes or include info is true
            {
                g_cachedParameter[objectID][paramTag].objectID = objectID
                g_cachedParameter[objectID][paramTag].title = newTitle
                displayName = newTitle
            }

            // quantisation
            var defaultQuantisation = undefined

            // we need a default quantisation because we can't find out the real value before Cubase 15
            if (paramTag == PARAMTAG.EQ1_TYPE || paramTag == PARAMTAG.EQ4_TYPE)
                defaultQuantisation = 8
            else if (paramTag == PARAMTAG.EQ2_TYPE || paramTag == PARAMTAG.EQ3_TYPE || paramTag == PARAMTAG.EQ1_ON || paramTag == PARAMTAG.EQ2_ON || paramTag == PARAMTAG.EQ3_ON || paramTag == PARAMTAG.EQ4_ON)
                defaultQuantisation = 2

            // only check quantisation for discrete parameters
            type = g_remote.mixConsole.getParameterProcessValueType ? g_remote.mixConsole.getParameterProcessValueType(activeMapping, objectID, paramTag) : undefined

            var newQuantisation = type == "discrete" ? getQuantization(activeMapping, objectID, paramTag, defaultQuantisation) : 0

            if (g_cachedParameter[objectID][paramTag].quantisation != newQuantisation || includeInfo) // we only want to send the quantisation when i changes or inlcude info is true
            {
                g_cachedParameter[objectID][paramTag].objectID = objectID
                g_cachedParameter[objectID][paramTag].quantisation = newQuantisation
                quantisation = newQuantisation;
            }

            // value
            var newValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)

            if (g_cachedParameter[objectID][paramTag].value != newValue || includeInfo)
            {
                g_cachedParameter[objectID][paramTag].objectID = objectID
                g_cachedParameter[objectID][paramTag].value = newValue
                value = newValue
            }

            // display value
            var newDisplayValue = g_remote.mixConsole.getParameterDisplayValue(activeMapping, objectID, paramTag)

            if (g_cachedParameter[objectID][paramTag].displayValue != newDisplayValue || includeInfo)
            {
                g_cachedParameter[objectID][paramTag].objectID = objectID
                g_cachedParameter[objectID][paramTag].displayValue = newDisplayValue
                displayValue = newDisplayValue
            }

            // default value
            if (g_remote.mixConsole.getParameterDefaultProcessValue)
            {
                var newDefaultValue = g_remote.mixConsole.getParameterDefaultProcessValue(activeMapping, objectID, paramTag)

                if (g_cachedParameter[objectID][paramTag].defaultValue != newDefaultValue || includeInfo) // we only want to send the default value when it changes or include info is true
                {
                    g_cachedParameter[objectID][paramTag].objectID = objectID
                    g_cachedParameter[objectID][paramTag].defaultValue = newDefaultValue
                    defaultValue = newDefaultValue;
                }
            }

            // hack: comp on, needs to be reversed since it is using the bypass parameter
            if (name == 'compOn')
            {
                if (displayName == "Bypass")
                    displayName = "Comp On"
                if (value !== undefined)
                    value = 1.0 - value
                if (displayValue === 'On')
                    displayValue = 'Off'
                else if (displayValue === 'Off')
                    displayValue = 'On'
                if (defaultValue != undefined)
                    defaultValue = 1.0 - defaultValue
            }

            // send midi data to OSD
            if (displayName !== undefined || quantisation !== undefined || value !== undefined || displayValue !== undefined || defaultValue !== undefined) {
                handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[parentID], name, {
                    name: displayName,
                    quantisation: quantisation,
                    value: value,
                    display_value: displayValue,
                    default_value: defaultValue
                })
            }
        }

        // filter
        if (g_filterTagToParamMap[objectID])
        {
            if (g_filterTagToParamMap[objectID][paramTag])
            {
                handleParameterValueChange(objectID, paramTag, g_filterTagToParamMap[objectID][paramTag], includeInfo)
            }
        }

        // equalizer
        else if (g_eqTagToParamMap[objectID])
        {
            if (g_eqTagToParamMap[objectID][paramTag])
            {
                handleParameterValueChange(objectID, paramTag, g_eqTagToParamMap[objectID][paramTag], includeInfo)
            }
        }

        // compressors
        else if (g_compTagToParamMap[objectID])
        {
            if (g_compTagToParamMap[objectID][paramTag])
            {
                handleParameterValueChange(objectID, paramTag, g_compTagToParamMap[objectID][paramTag], includeInfo)
            }
        }

        // handle rest of the paramters
        else
        {
            // handle parameters
            switch (paramTag) {
                case PARAMTAG.NAME:
                    var nameStrValue = g_remote.mixConsole.getParameterDisplayValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'name', nameStrValue)
                    break
                case PARAMTAG.VOLUME:
                    var volumeValueDbStr = g_remote.mixConsole.getParameterDisplayValue(activeMapping, objectID, paramTag)
                    var volumeValueDb = -Infinity
                    if (volumeValueDbStr !== '-oo') {
                        volumeValueDb = parseFloat(volumeValueDbStr)
                    }
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'volume', volumeValueDb)
                    break
                case PARAMTAG.MUTE:
                    var muteValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'mute', Boolean(muteValue))
                    break
                case PARAMTAG.SOLO:
                    var soloValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'solo', Boolean(soloValue))
                    break
                case PARAMTAG.SELECTED:
                    var selectedValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'selected', Boolean(selectedValue))
                    break
                // case PARAMTAG.METER:
                //     var meterValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, PARAMTAG.METER)
                //     var peakValue = meterValue * Math.sqrt(2); // C1 currently expects a peak value
                //     if (validParamValue(peakValue)) {
                //         handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[objectID], 'meter', [peakValue])
                //     }
                //     break
                case PARAMTAG.PAN:
                    var panValue = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[g_childToParentMap[objectID]], 'pan', panValue)
                    break
                case PARAMTAG.SEND_ON:
                    var parentID = g_childToParentMap[objectID]
                    var sendSlotIDs = g_parentToSendsMap[parentID]
                    var sendSlot = 'send' + objectID2SendSlotNumber(sendSlotIDs, objectID).toString() + 'On'
                    var sendOn = g_remote.mixConsole.getParameterProcessValue(activeMapping, objectID, paramTag)
                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[parentID], sendSlot, Boolean(sendOn))
                    break
                case PARAMTAG.SEND_LEVEL:
                    var parentID = g_childToParentMap[objectID]
                    var sendSlotIDs = g_parentToSendsMap[parentID]
                    var sendSlot = 'send' + objectID2SendSlotNumber(sendSlotIDs, objectID).toString()

                    var sendValueDbStr = g_remote.mixConsole.getParameterDisplayValue(activeMapping, objectID, paramTag)
                    var sendValueDb = -Infinity
                    if (sendValueDbStr !== '-oo') {
                        sendValueDb = parseFloat(sendValueDbStr)
                    }

                    handleTrackPropertyChange(activeDevice, g_cachedTrackInfo[parentID], sendSlot, sendValueDb)
                    break;

                default:
                    // Do nothing
            }
        }
    }
    else {
        //console.log('parameterChangeCallback for unknown objectID')
    }
}

function getQuantization(activeMapping, objectID, paramTag, fallBack)
{
    if (g_remote.mixConsole.convertParameterProcessValueToPlain)
    {
        var minPlain = g_remote.mixConsole.convertParameterProcessValueToPlain(activeMapping, objectID, paramTag, 0.0)
        var maxPlain = g_remote.mixConsole.convertParameterProcessValueToPlain(activeMapping, objectID, paramTag, 1.0)

        return  Math.round(maxPlain - minPlain + 1)
    }

    return fallBack;
}

function pageIdleCallback(activeDevice, activeMapping) {
    if (g_enabled) {
        batchSendChangedMeters(activeDevice, activeMapping);
    }
}

module.exports = {
    init
}
