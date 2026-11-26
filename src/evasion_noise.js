// src/evasion_noise.js — Opt-in Canvas & AudioContext Anti-Hash Clustering (stealth="aggressive")
// Never mutates live visible canvases or transparent pixels (alpha === 0) or tiny <16px probe canvases.
(() => {
    'use strict';

    const origToString = Function.prototype.toString;
    const patched = new WeakMap();
    const mask = (fn, name) => {
        patched.set(fn, name);
        try { Object.defineProperty(fn, 'name', { value: name, configurable: true }); } catch (_) {}
        return fn;
    };
    const wrappedToString = ({
        toString() {
            if (this === wrappedToString) return 'function toString() { [native code] }';
            if (patched.has(this)) return `function ${patched.get(this)}() { [native code] }`;
            return origToString.call(this);
        }
    }).toString;
    patched.set(wrappedToString, 'toString');
    try {
        Object.defineProperty(Function.prototype, 'toString', {
            value: wrappedToString, writable: true, configurable: true, enumerable: false,
        });
    } catch (_) {}

    try {
        const shift = (Math.random() < 0.5) ? 1 : -1;
        const origGetImageData = CanvasRenderingContext2D.prototype.getImageData;
        const origToDataURL = HTMLCanvasElement.prototype.toDataURL;

        const perturb = (buf) => {
            for (let i = 0; i + 3 < buf.length; i += 400) {
                if (buf[i + 3] > 0) {
                    const v = buf[i];
                    buf[i] = (v === 255 && shift > 0) ? 254 : (v === 0 && shift < 0) ? 1 : (v + shift);
                }
            }
        };

        const spoofedGetImageData = mask(({
            getImageData(sx, sy, sw, sh) {
                const img = origGetImageData.call(this, sx, sy, sw, sh);
                if (sw * sh >= 16) perturb(img.data);
                return img;
            }
        }).getImageData, 'getImageData');
        CanvasRenderingContext2D.prototype.getImageData = spoofedGetImageData;

        const spoofedToDataURL = mask(({
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
                                perturb(img.data);
                                cctx.putImageData(img, 0, 0);
                                return origToDataURL.call(clone, type, quality);
                            }
                        }
                    } catch (_) {}
                }
                return origToDataURL.call(this, type, quality);
            }
        }).toDataURL, 'toDataURL');
        HTMLCanvasElement.prototype.toDataURL = spoofedToDataURL;
    } catch (_) {}

    try {
        if (window.AudioBuffer) {
            const audioSalt = (Math.random() - 0.5) * 0.0000001;
            const seen = new WeakSet();
            const origGetChannelData = AudioBuffer.prototype.getChannelData;
            const spoofedGetChannelData = mask(({
                getChannelData(channel) {
                    const data = origGetChannelData.call(this, channel);
                    if (!seen.has(data)) {
                        seen.add(data);
                        for (let i = 0; i < data.length; i += 256) data[i] = data[i] + audioSalt;
                    }
                    return data;
                }
            }).getChannelData, 'getChannelData');
            AudioBuffer.prototype.getChannelData = spoofedGetChannelData;
        }
    } catch (_) {}
})();
