// src/evasion.js — Tier-1+ Native Prototype Stealth Evasion Script
(() => {
    'use strict';

    // ───────────────────────────── 1. Core Native Engine ─────────────────────────────
    // Tracks custom functions and makes them indistinguishable from V8 native methods/getters.
    // All patched functions use concise method syntax so `'prototype' in fn === false` (non-constructable).
    const nativeFunctions = new WeakMap();
    const origToString = Function.prototype.toString;

    function makeNative(fn, name) {
        const fnName = name !== undefined ? name : (fn.name || '');
        nativeFunctions.set(fn, fnName);
        try {
            Object.defineProperty(fn, 'name', { value: fnName, configurable: true });
        } catch (_) {}
        return fn;
    }

    const customToString = ({
        toString() {
            if (this === customToString) {
                return 'function toString() { [native code] }';
            }
            if (nativeFunctions.has(this)) {
                const name = nativeFunctions.get(this);
                return name ? `function ${name}() { [native code] }` : 'function () { [native code] }';
            }
            return origToString.call(this);
        }
    }).toString;

    makeNative(customToString, 'toString');
    try {
        Object.defineProperty(Function.prototype, 'toString', {
            value: customToString,
            writable: true,
            configurable: true,
            enumerable: false,
        });
    } catch (_) {}

    function defineProtoGetter(proto, prop, getterFn) {
        if (!proto) return;
        const getter = ({ [prop]() { return getterFn.call(this); } })[prop];
        makeNative(getter, `get ${prop}`);
        Object.defineProperty(proto, prop, {
            get: getter,
            set: undefined,
            enumerable: true,
            configurable: true,
        });
    }

    const navProto = Object.getPrototypeOf(navigator) || Navigator.prototype;

    // ───────────────────────────── 2. Navigator.webdriver (Real Descriptor Spoofing) ─────────────────────────────
    // In genuine Chrome:
    // 1. 'webdriver' in Navigator.prototype === true
    // 2. Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver').get returns false & has no .prototype
    // 3. Object.getOwnPropertyNames(navigator).length === 0
    try {
        defineProtoGetter(navProto, 'webdriver', () => false);
        if (Object.prototype.hasOwnProperty.call(navigator, 'webdriver')) {
            delete navigator.webdriver;
        }
    } catch (_) {}

    // ───────────────────────────── 3. window.chrome Environment ─────────────────────────────
    if (!window.chrome) {
        window.chrome = {};
    }

    const csiFn = makeNative(({ csi() {} }).csi, 'csi');
    const loadTimesFn = makeNative(({
        loadTimes() {
            const now = Date.now() / 1000;
            return {
                commitLoadTime: now - 0.2,
                connectionInfo: 'http/1.1',
                finishDocumentLoadTime: now - 0.05,
                finishLoadTime: now,
                firstPaintAfterLoadTime: 0,
                firstPaintTime: now - 0.15,
                navigationType: 'Other',
                requestTime: now - 0.3,
                startLoadTime: now - 0.3,
                wasAlternateProtocolAvailable: false,
                wasFetchedViaSpdy: false,
                wasNpnNegotiated: false,
                npnNegotiatedProtocol: 'unknown'
            };
        }
    }).loadTimes, 'loadTimes');

    window.chrome.csi = window.chrome.csi || csiFn;
    window.chrome.loadTimes = window.chrome.loadTimes || loadTimesFn;

    window.chrome.app = window.chrome.app || {
        isInstalled: false,
        InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
        RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
        getDetails: makeNative(({ getDetails() { return null; } }).getDetails, 'getDetails'),
        getIsInstalled: makeNative(({ getIsInstalled() { return false; } }).getIsInstalled, 'getIsInstalled'),
        installState: makeNative(({ installState() { return 'not_installed'; } }).installState, 'installState'),
    };

    window.chrome.runtime = window.chrome.runtime || {
        PlatformOs: { MAC: 'mac', WIN: 'win', ANDROID: 'android', CROS: 'cros', LINUX: 'linux', OPENBSD: 'openbsd' },
        PlatformArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
        PlatformNaclArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
        RequestUpdateCheckStatus: { THROTTLED: 'throttled', NO_UPDATE: 'no_update', UPDATE_AVAILABLE: 'update_available' },
        OnInstalledReason: { INSTALL: 'install', UPDATE: 'update', CHROME_UPDATE: 'chrome_update', SHARED_MODULE_UPDATE: 'shared_module_update' },
        OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
        connect: makeNative(({ connect() {} }).connect, 'connect'),
        sendMessage: makeNative(({ sendMessage() {} }).sendMessage, 'sendMessage'),
    };

    // ───────────────────────────── 4. Permissions Query Consistency (Prototype Patch) ─────────────────────────────
    try {
        if (window.Permissions && Permissions.prototype && Permissions.prototype.query) {
            const origQuery = Permissions.prototype.query;
            const proxiedQuery = ({
                query(parameters) {
                    if (parameters && parameters.name === 'notifications') {
                        const state = (typeof Notification !== 'undefined' && Notification.permission === 'default')
                            ? 'prompt'
                            : (typeof Notification !== 'undefined' ? Notification.permission : 'prompt');
                        return Promise.resolve({
                            state: state,
                            name: 'notifications',
                            onchange: null
                        });
                    }
                    return origQuery.call(this, parameters);
                }
            }).query;
            makeNative(proxiedQuery, 'query');
            Permissions.prototype.query = proxiedQuery;
        }
    } catch (_) {}

    // ───────────────────────────── 5. PluginArray & MimeTypeArray (Prototype Patch) ─────────────────────────────
    if (!navigator.plugins || navigator.plugins.length === 0 || !(navigator.plugins instanceof PluginArray)) {
        const pluginData = [
            { name: 'PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format', mime: 'application/pdf' },
            { name: 'Chrome PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format', mime: 'application/pdf' },
            { name: 'Chromium PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format', mime: 'application/pdf' },
            { name: 'Microsoft Edge PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format', mime: 'application/pdf' },
            { name: 'WebKit built-in PDF', filename: 'internal-pdf-viewer', description: 'Portable Document Format', mime: 'application/pdf' }
        ];

        try {
            const plugins = Object.create(PluginArray.prototype);
            const mimeTypes = Object.create(MimeTypeArray.prototype);

            pluginData.forEach((p, idx) => {
                const plugin = Object.create(Plugin.prototype);
                const mimeType = Object.create(MimeType.prototype);

                Object.defineProperties(mimeType, {
                    type: { value: p.mime, enumerable: true },
                    suffixes: { value: 'pdf', enumerable: true },
                    description: { value: p.description, enumerable: true },
                    enabledPlugin: { value: plugin, enumerable: true },
                });

                Object.defineProperties(plugin, {
                    name: { value: p.name, enumerable: true },
                    filename: { value: p.filename, enumerable: true },
                    description: { value: p.description, enumerable: true },
                    length: { value: 1, enumerable: true },
                    0: { value: mimeType, enumerable: true },
                });

                plugins[idx] = plugin;
                plugins[p.name] = plugin;
                mimeTypes[idx] = mimeType;
                mimeTypes[p.mime] = mimeType;
            });

            const pluginItem = makeNative(({ item(i) { return plugins[i] || null; } }).item, 'item');
            const pluginNamedItem = makeNative(({ namedItem(name) { return plugins[name] || null; } }).namedItem, 'namedItem');
            const pluginRefresh = makeNative(({ refresh() {} }).refresh, 'refresh');

            Object.defineProperties(plugins, {
                length: { value: pluginData.length, enumerable: false },
                item: { value: pluginItem, enumerable: false },
                namedItem: { value: pluginNamedItem, enumerable: false },
                refresh: { value: pluginRefresh, enumerable: false },
            });

            const mimeItem = makeNative(({ item(i) { return mimeTypes[i] || null; } }).item, 'item');
            const mimeNamedItem = makeNative(({ namedItem(name) { return mimeTypes[name] || null; } }).namedItem, 'namedItem');

            Object.defineProperties(mimeTypes, {
                length: { value: pluginData.length, enumerable: false },
                item: { value: mimeItem, enumerable: false },
                namedItem: { value: mimeNamedItem, enumerable: false },
            });

            defineProtoGetter(navProto, 'plugins', () => plugins);
            defineProtoGetter(navProto, 'mimeTypes', () => mimeTypes);
        } catch (_) {}
    }

    // ───────────────────────────── 6. WebGL Vendor & Renderer Spoofing ─────────────────────────────
    const patchWebGLProto = (proto) => {
        if (!proto || !proto.getParameter) return;
        const origGetParam = proto.getParameter;
        const spoofedGetParam = ({
            getParameter(param) {
                // UNMASKED_VENDOR_WEBGL
                if (param === 37445) {
                    return 'Google Inc. (NVIDIA)';
                }
                // UNMASKED_RENDERER_WEBGL
                if (param === 37446) {
                    return 'ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)';
                }
                return origGetParam.call(this, param);
            }
        }).getParameter;
        makeNative(spoofedGetParam, 'getParameter');
        proto.getParameter = spoofedGetParam;
    };

    if (window.WebGLRenderingContext) patchWebGLProto(WebGLRenderingContext.prototype);
    if (window.WebGL2RenderingContext) patchWebGLProto(WebGL2RenderingContext.prototype);

    // ───────────────────────────── 7. Safe Canvas Micro-Noise (Anti-Hash Clustering) ─────────────────────────────
    // Per-session deterministic noise that NEVER alters transparent pixels or tiny 1x1 probe canvases,
    // and NEVER mutates the live visible canvas in place.
    try {
        const canvasChannelShift = (Math.random() < 0.5) ? 1 : -1;
        const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
        const origToDataURL = HTMLCanvasElement.prototype.toDataURL;

        function perturbNonTransparentPixels(buf) {
            for (let i = 0; i + 3 < buf.length; i += 400) {
                if (buf[i + 3] > 0) {
                    const v = buf[i];
                    buf[i] = (v === 255 && canvasChannelShift > 0) ? 254
                           : (v === 0 && canvasChannelShift < 0) ? 1
                           : (v + canvasChannelShift);
                }
            }
        }

        const spoofedGetImageData = ({
            getImageData(sx, sy, sw, sh) {
                const img = origGetImageData.call(this, sx, sy, sw, sh);
                if (sw * sh >= 16) {
                    perturbNonTransparentPixels(img.data);
                }
                return img;
            }
        }).getImageData;
        makeNative(spoofedGetImageData, 'getImageData');
        CanvasRenderingContext2D.prototype.getImageData = spoofedGetImageData;

        const spoofedToDataURL = ({
            toDataURL(type, quality) {
                if (this.width >= 16 && this.height >= 16) {
                    try {
                        const ctx = this.getContext('2d');
                        if (ctx) {
                            const clone = document.createElement('canvas');
                            clone.width = this.width;
                            clone.height = this.height;
                            const cctx = clone.getContext('2d');
                            if (cctx) {
                                cctx.drawImage(this, 0, 0);
                                const img = origGetImageData.call(cctx, 0, 0, clone.width, clone.height);
                                perturbNonTransparentPixels(img.data);
                                cctx.putImageData(img, 0, 0);
                                return origToDataURL.call(clone, type, quality);
                            }
                        }
                    } catch (_) {}
                }
                return origToDataURL.call(this, type, quality);
            }
        }).toDataURL;
        makeNative(spoofedToDataURL, 'toDataURL');
        HTMLCanvasElement.prototype.toDataURL = spoofedToDataURL;
    } catch (_) {}

    // ───────────────────────────── 8. AudioContext Fingerprint Micro-Noise ─────────────────────────────
    try {
        if (window.AudioBuffer) {
            const audioSalt = (Math.random() - 0.5) * 0.0000001;
            const noisifiedBuffers = new WeakSet();
            const origGetChannelData = AudioBuffer.prototype.getChannelData;
            const spoofedGetChannelData = ({
                getChannelData(channel) {
                    const data = origGetChannelData.call(this, channel);
                    if (!noisifiedBuffers.has(data)) {
                        noisifiedBuffers.add(data);
                        for (let i = 0; i < data.length; i += 256) {
                            data[i] = data[i] + audioSalt;
                        }
                    }
                    return data;
                }
            }).getChannelData;
            makeNative(spoofedGetChannelData, 'getChannelData');
            AudioBuffer.prototype.getChannelData = spoofedGetChannelData;
        }
    } catch (_) {}

    // ───────────────────────────── 9. Hardware & Language Consistency (Prototype Patch) ─────────────────────────────
    try {
        if (!navigator.languages || navigator.languages.length === 0) {
            const langs = Object.freeze(['en-US', 'en']);
            defineProtoGetter(navProto, 'languages', () => langs);
        }
        if (!navigator.hardwareConcurrency || navigator.hardwareConcurrency < 2) {
            defineProtoGetter(navProto, 'hardwareConcurrency', () => 8);
        }
        if (!navigator.deviceMemory || navigator.deviceMemory < 2) {
            defineProtoGetter(navProto, 'deviceMemory', () => 8);
        }
    } catch (_) {}

    // ───────────────────────────── 10. SpeechSynthesis Voices (Prototype Patch) ─────────────────────────────
    try {
        if (window.SpeechSynthesis && SpeechSynthesis.prototype && SpeechSynthesis.prototype.getVoices) {
            const origGetVoices = SpeechSynthesis.prototype.getVoices;
            const voiceProto = window.SpeechSynthesisVoice ? SpeechSynthesisVoice.prototype : Object.prototype;
            const makeVoice = (v) => {
                const obj = Object.create(voiceProto);
                for (const [k, val] of Object.entries(v)) {
                    Object.defineProperty(obj, k, { value: val, enumerable: true });
                }
                return obj;
            };
            const fakeVoices = [
                makeVoice({ default: true, lang: 'en-US', localService: true, name: 'Microsoft David - English (United States)', voiceURI: 'Microsoft David - English (United States)' }),
                makeVoice({ default: false, lang: 'en-US', localService: true, name: 'Microsoft Zira - English (United States)', voiceURI: 'Microsoft Zira - English (United States)' }),
                makeVoice({ default: false, lang: 'en-US', localService: true, name: 'Google US English', voiceURI: 'Google US English' })
            ];
            const getVoices = ({
                getVoices() {
                    const real = origGetVoices.call(this);
                    return (real && real.length > 0) ? real : fakeVoices;
                }
            }).getVoices;
            makeNative(getVoices, 'getVoices');
            SpeechSynthesis.prototype.getVoices = getVoices;
        }
    } catch (_) {}

    // ───────────────────────────── 11. MediaDevices Enumerate (Prototype Patch) ─────────────────────────────
    try {
        if (window.MediaDevices && MediaDevices.prototype && MediaDevices.prototype.enumerateDevices) {
            const origEnum = MediaDevices.prototype.enumerateDevices;
            const fakeDevices = [
                { deviceId: 'default', kind: 'audioinput', label: '', groupId: 'group_default_audio' },
                { deviceId: 'default', kind: 'videoinput', label: '', groupId: 'group_default_video' },
                { deviceId: 'default', kind: 'audiooutput', label: '', groupId: 'group_default_speaker' }
            ];
            const enumerateDevices = ({
                enumerateDevices() {
                    return origEnum.call(this).then(list => (list && list.length > 0) ? list : fakeDevices).catch(() => fakeDevices);
                }
            }).enumerateDevices;
            makeNative(enumerateDevices, 'enumerateDevices');
            MediaDevices.prototype.enumerateDevices = enumerateDevices;
        }
    } catch (_) {}

    // ───────────────────────────── 12. Network Information (Prototype Patch) ─────────────────────────────
    try {
        if (!navigator.connection) {
            const connection = {
                downlink: 10,
                effectiveType: '4g',
                rtt: 50,
                saveData: false,
                onchange: null
            };
            defineProtoGetter(navProto, 'connection', () => connection);
        }
    } catch (_) {}

    // ───────────────────────────── 13. Screen & Outer Dimensions Sync ─────────────────────────────
    try {
        if (window.outerWidth === 0 || window.outerWidth === window.innerWidth) {
            Object.defineProperty(window, 'outerWidth', {
                get: makeNative(({ outerWidth() { return window.innerWidth + 16; } }).outerWidth, 'get outerWidth'),
                configurable: true
            });
        }
        if (window.outerHeight === 0 || window.outerHeight === window.innerHeight) {
            Object.defineProperty(window, 'outerHeight', {
                get: makeNative(({ outerHeight() { return window.innerHeight + 87; } }).outerHeight, 'get outerHeight'),
                configurable: true
            });
        }
    } catch (_) {}

    // ───────────────────────────── 14. Battery API (Prototype Patch — Zero navigator own props) ─────────────────────────────
    try {
        if (navProto && typeof navProto.getBattery === 'function') {
            const origGetBattery = navProto.getBattery;
            const fakeBattery = {
                charging: true,
                chargingTime: 0,
                dischargingTime: Infinity,
                level: 1.0,
                onchargingchange: null,
                onchargingtimechange: null,
                ondischargingtimechange: null,
                onlevelchange: null
            };
            const getBattery = ({
                getBattery() {
                    return origGetBattery.call(this).catch(() => fakeBattery);
                }
            }).getBattery;
            makeNative(getBattery, 'getBattery');
            navProto.getBattery = getBattery;
        }
    } catch (_) {}
})();
