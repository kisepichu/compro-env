// ==UserScript==
// @name         ce submit helper
// @namespace    https://github.com/kisepichu/compro-env
// @version      1.1
// @description  Auto-fill AtCoder submit form from ce submit URL fragment
// @author       kisepichu
// @match        https://atcoder.jp/contests/*/submit*
// @grant        none
// @run-at       document-idle
// ==/UserScript==

(function () {
    'use strict';

    const hash = location.hash;
    if (!hash.startsWith('#ce=')) return;

    // --- 1. Decode payload ---
    const payloadB64 = hash.slice('#ce='.length);
    // Convert URL-safe base64 (RFC 4648 §5) to standard base64
    const standardB64 = payloadB64.replace(/-/g, '+').replace(/_/g, '/');
    let payload;
    try {
        // atob() produces a Latin-1 binary string; use TextDecoder to handle UTF-8.
        const binary = atob(standardB64);
        const bytes = Uint8Array.from(binary, c => c.charCodeAt(0));
        payload = JSON.parse(new TextDecoder().decode(bytes));
    } catch (e) {
        console.error('[ce] Failed to decode #ce= fragment:', e);
        return;
    }
    const { lang_id: langId, source } = payload;
    if (!langId || source == null) {
        console.error('[ce] Payload missing lang_id or source');
        return;
    }

    // --- 2. Get taskScreenName from query string ---
    const taskScreenName = new URLSearchParams(location.search).get('taskScreenName');

    // --- 3. Clean URL fragment immediately ---
    history.replaceState(null, '', location.pathname + location.search);

    // --- 4. Select task and language (retry until select2 is ready) ---
    let attempts = 0;
    const MAX_ATTEMPTS = 20;
    const INTERVAL_MS = 300;

    const timer = setInterval(function () {
        attempts++;

        const taskSelect = document.querySelector('#select-task');
        if (!taskSelect) {
            if (attempts >= MAX_ATTEMPTS) {
                clearInterval(timer);
                console.error('[ce] Timed out waiting for #select-task');
            }
            return;
        }

        // Step A: select task (reveals the per-task language div)
        if (taskScreenName) {
            $(taskSelect).val(taskScreenName).trigger('change');
        }

        // Step B: wait for the language div to become visible
        const langDivId = taskScreenName ? `select-lang-${taskScreenName}` : null;
        const langDiv = langDivId
            ? document.getElementById(langDivId)
            : document.querySelector('[id^="select-lang-"]');

        if (!langDiv || langDiv.style.display === 'none') {
            if (attempts >= MAX_ATTEMPTS) {
                clearInterval(timer);
                console.error('[ce] Timed out waiting for language div to appear');
            }
            return;
        }

        clearInterval(timer);

        // Step C: select language
        const langSelect = langDiv.querySelector('select');
        if (langSelect) {
            $(langSelect).val(langId).trigger('change');
        }

        // --- 5. Inject source code ---
        // AtCoder uses Ace editor (div#editor). ace.edit() returns the existing instance.
        if (typeof ace !== 'undefined') {
            const editor = ace.edit('editor');
            editor.setValue(source, -1); // -1 = place cursor at start
            console.info('[ce] Source injected via ace.edit().setValue()');
        } else {
            // Fallback: write to the hidden textarea (AtCoder syncs it on submit)
            const sourceArea = document.getElementById('plain-textarea');
            if (sourceArea) {
                sourceArea.value = source;
                sourceArea.dispatchEvent(new Event('input', { bubbles: true }));
                sourceArea.dispatchEvent(new Event('change', { bubbles: true }));
                console.warn('[ce] Ace not found; injected via #plain-textarea');
            } else {
                console.error('[ce] Could not find any editor to inject source');
            }
        }

        console.info('[ce] Submit form filled successfully');
    }, INTERVAL_MS);
})();
