// src/evasion.js — Stealth Engine v2: Lazy, Non-Destructive Prototype Stealth
// Design invariants:
// 1. NEVER replace genuine C++ native getters/methods when Chromium already returns the authentic value
//    (e.g. `navigator.webdriver === false` via `--disable-blink-features=AutomationControlled`),
//    eliminating cross-realm `iframe.contentWindow.Function.prototype.toString` lie-detector flags.
// 2. ONLY patch `Function.prototype.toString` lazily when a JS shim is actually installed.
// 3. ONLY spoof WebGL vendor/renderer when a software renderer (SwiftShader/llvmpipe) is detected,
//    choosing an OS-consistent GPU profile matching `navigator.userAgent` (no Direct3D11 on macOS/Linux).
// 4. Keep `Object.getOwnPropertyNames(navigator)` and `Object.getOwnPropertyNames(navigator.permissions)` empty.
(() => {
    'use strict';

    // ───────────────────────────── 1. Lazy Native Function Masker ─────────────────────────────
    const nativeFunctions = new WeakMap();
    const origToString = Function.prototype.toString;
    let toStringPatched = false;

    function ensureToStringPatched() {
        if (toStringPatched) return;
        toStringPatched = true;
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
        nativeFunctions.set(customToString, 'toString');
        try {
            Object.defineProperty(Function.prototype, 'toString', {
                value: customToString,
                writable: true,
                configurable: true,
                enumerable: false,
            });
        } catch (_) {}
    }

    function makeNative(fn, name) {
        ensureToStringPatched();
        const fnName = name !== undefined ? name : (fn.name || '');
        nativeFunctions.set(fn, fnName);
        try {
            Object.defineProperty(fn, 'name', { value: fnName, configurable: true });
        } catch (_) {}
        return fn;
    }

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

    // ───────────────────────────── 2. Navigator.webdriver (Zero-Lie Lazy Guard) ─────────────────────────────
    // When `--disable-blink-features=AutomationControlled` is active, V8 already reports
    // `navigator.webdriver === false` via its genuine C++ getter — leave it untouched so
    // cross-realm `iframe.contentWindow.Function.prototype.toString.call(getter)` sees real native code.
    try {
        if (navigator.webdriver !== false) {
            defineProtoGetter(navProto, 'webdriver', () => false);
        }
        if (Object.prototype.hasOwnProperty.call(navigator, 'webdriver')) {
            delete navigator.webdriver;
        }
    } catch (_) {}

    // ───────────────────────────── 3. window.chrome Environment ─────────────────────────────
    if (!window.chrome) {
        window.chrome = {};
    }
    if (!window.chrome.csi) {
        window.chrome.csi = makeNative(({ csi() {} }).csi, 'csi');
    }
    if (!window.chrome.loadTimes) {
        window.chrome.loadTimes = makeNative(({
            loadTimes() {
                const now = Date.now() / 1000;
                return {
                    commitLoadTime: now - 0.2,
                    connectionInfo: 'h2',
                    finishDocumentLoadTime: now - 0.05,
                    finishLoadTime: now,
                    firstPaintAfterLoadTime: 0,
                    firstPaintTime: now - 0.15,
                    navigationType: 'Other',
                    requestTime: now - 0.3,
                    startLoadTime: now - 0.3,
                    wasAlternateProtocolAvailable: false,
                    wasFetchedViaSpdy: true,
                    wasNpnNegotiated: true,
                    npnNegotiatedProtocol: 'h2'
                };
            }
        }).loadTimes, 'loadTimes');
    }
    if (!window.chrome.app) {
        window.chrome.app = {
            isInstalled: false,
            InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
            RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
            getDetails: makeNative(({ getDetails() { return null; } }).getDetails, 'getDetails'),
            getIsInstalled: makeNative(({ getIsInstalled() { return false; } }).getIsInstalled, 'getIsInstalled'),
            installState: makeNative(({ installState() { return 'not_installed'; } }).installState, 'installState'),
        };
    }
    if (!window.chrome.runtime) {
        window.chrome.runtime = {
            PlatformOs: { MAC: 'mac', WIN: 'win', ANDROID: 'android', CROS: 'cros', LINUX: 'linux', OPENBSD: 'openbsd' },
            PlatformArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
            PlatformNaclArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
            RequestUpdateCheckStatus: { THROTTLED: 'throttled', NO_UPDATE: 'no_update', UPDATE_AVAILABLE: 'update_available' },
            OnInstalledReason: { INSTALL: 'install', UPDATE: 'update', CHROME_UPDATE: 'chrome_update', SHARED_MODULE_UPDATE: 'shared_module_update' },
            OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' },
            connect: makeNative(({ connect() {} }).connect, 'connect'),
            sendMessage: makeNative(({ sendMessage() {} }).sendMessage, 'sendMessage'),
        };
    }

    // ───────────────────────────── 4. Permissions Query Consistency ─────────────────────────────
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

    // ───────────────────────────── 5. PluginArray & MimeTypeArray (Only When Empty) ─────────────────────────────
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

    // ───────────────────────────── 6. OS-Consistent WebGL Fallback (Software Renderer Only) ─────────────────────────────
    // Preserves genuine hardware GPU strings on real machines; replaces ONLY SwiftShader / llvmpipe / software renderers.
    const uaStr = navigator.userAgent || '';
    const webglFallback = uaStr.includes('Mac OS X')
        ? { vendor: 'Google Inc. (Apple)', renderer: 'ANGLE (Apple, ANGLE Metal Renderer: Apple M2, Unspecified Version)' }
        : uaStr.includes('Linux') && !uaStr.includes('Android')
            ? { vendor: 'Google Inc. (Intel)', renderer: 'ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)' }
            : { vendor: 'Google Inc. (NVIDIA)', renderer: 'ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0, D3D11)' };

    const isSoftwareRenderer = (val) => {
        if (!val || typeof val !== 'string') return true;
        const low = val.toLowerCase();
        return low.includes('swiftshader') || low.includes('llvmpipe') || low.includes('softpipe') || low.includes('basic render driver');
    };

    const patchWebGLProto = (proto) => {
        if (!proto || !proto.getParameter) return;
        const origGetParam = proto.getParameter;
        const guardedGetParam = ({
            getParameter(param) {
                const real = origGetParam.call(this, param);
                if (param === 37445) {
                    const realRenderer = origGetParam.call(this, 37446);
                    if (isSoftwareRenderer(realRenderer) || isSoftwareRenderer(real)) {
                        return webglFallback.vendor;
                    }
                    return real;
                }
                if (param === 37446) {
                    if (isSoftwareRenderer(real)) {
                        return webglFallback.renderer;
                    }
                    return real;
                }
                return real;
            }
        }).getParameter;
        makeNative(guardedGetParam, 'getParameter');
        proto.getParameter = guardedGetParam;
    };

    if (window.WebGLRenderingContext) patchWebGLProto(WebGLRenderingContext.prototype);
    if (window.WebGL2RenderingContext) patchWebGLProto(WebGL2RenderingContext.prototype);

    // ───────────────────────────── 7. Hardware & Language Fallbacks (Only When Missing) ─────────────────────────────
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

    // ───────────────────────────── 8. SpeechSynthesis Voices (Fallback Only When Empty) ─────────────────────────────
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

    // ───────────────────────────── 9. MediaDevices Enumerate (Fallback Only When Empty) ─────────────────────────────
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

    // ───────────────────────────── 10. Network Information (Only When Missing) ─────────────────────────────
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

    // ───────────────────────────── 11. Screen & Outer Dimensions Sync (Headless Guard) ─────────────────────────────
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

    // ───────────────────────────── 12. Battery API (Fallback Only On Rejection) ─────────────────────────────
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
