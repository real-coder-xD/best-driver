// src/evasion.js — injected via Page.addScriptToEvaluateOnNewDocument before any page script runs
(() => {
    // 1. Remove navigator.webdriver from both the prototype and the instance
    try { delete Object.getPrototypeOf(navigator).webdriver; } catch (e) {}
    try { delete navigator.webdriver; } catch (e) {}

    // 2. Full window.chrome object
    if (!window.chrome) {
        window.chrome = {};
    }
    window.chrome.runtime = window.chrome.runtime || {
        PlatformOs: { MAC: 'mac', WIN: 'win', ANDROID: 'android', CROS: 'cros', LINUX: 'linux', OPENBSD: 'openbsd' },
        PlatformArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
        PlatformNaclArch: { ARM: 'arm', X86_32: 'x86-32', X86_64: 'x86-64' },
        RequestUpdateCheckStatus: { THROTTLED: 'throttled', NO_UPDATE: 'no_update', UPDATE_AVAILABLE: 'update_available' },
        OnInstalledReason: { INSTALL: 'install', UPDATE: 'update', CHROME_UPDATE: 'chrome_update', SHARED_MODULE_UPDATE: 'shared_module_update' },
        OnRestartRequiredReason: { APP_UPDATE: 'app_update', OS_UPDATE: 'os_update', PERIODIC: 'periodic' }
    };
    window.chrome.app = window.chrome.app || {
        isInstalled: false,
        InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
        RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' }
    };
    window.chrome.csi = window.chrome.csi || function () {};
    window.chrome.loadTimes = window.chrome.loadTimes || function () {};

    // 3. Notification permission consistency
    const origQuery = window.navigator.permissions?.query;
    if (origQuery) {
        window.navigator.permissions.query = (parameters) => (
            parameters && parameters.name === 'notifications'
                ? Promise.resolve({ state: Notification.permission })
                : origQuery.call(window.navigator.permissions, parameters)
        );
    }

    // 4. PluginArray that satisfies instanceof checks
    if (!navigator.plugins || navigator.plugins.length === 0 || !(navigator.plugins instanceof PluginArray)) {
        const pluginData = [
            { name: 'PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'Chrome PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'Chromium PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'Microsoft Edge PDF Viewer', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
            { name: 'WebKit built-in PDF', filename: 'internal-pdf-viewer', description: 'Portable Document Format' }
        ];
        try {
            const plugins = Object.create(PluginArray.prototype);
            pluginData.forEach((p, idx) => {
                const plugin = Object.create(Plugin.prototype);
                Object.defineProperties(plugin, {
                    name: { value: p.name, enumerable: true },
                    filename: { value: p.filename, enumerable: true },
                    description: { value: p.description, enumerable: true },
                    length: { value: 0, enumerable: true },
                });
                plugins[idx] = plugin;
                plugins[p.name] = plugin;
            });
            Object.defineProperty(plugins, 'length', { value: pluginData.length, enumerable: false });
            Object.defineProperty(navigator, 'plugins', { get: () => plugins, configurable: true });
        } catch (e) {}
    }

    // 5. Languages
    Object.defineProperty(navigator, 'languages', {
        get: () => ['en-US', 'en'],
        configurable: true
    });
})();
