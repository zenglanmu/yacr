/*
 * The static-server plumbing of `scripts/web-ui-smoke.cjs`, kept in its own file
 * because `scripts/check-file-size.py` bounds every code file at 1000 lines.
 *
 * Nothing here is browser evidence: it starts/waits for/stops the `serve-web.py`
 * process the smoke measures, and fails loudly when the server never answers with
 * the built chrome bundle.
 */
'use strict';

const http = require('node:http');

/** Poll `url` until it serves the built bundle, or throw with the last reason. */
async function waitForServer(url, child, timeout, log) {
  const deadline = Date.now() + timeout;
  let lastError = 'no attempt was made';
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(
        `the static server exited with code ${child.exitCode} before answering ${url}\n` +
          log.slice(-20).join(''),
      );
    }
    try {
      const body = await httpGet(url, 2000);
      // A foreign server on this port could answer too; the bundle marker is what
      // says this is the page the script is about to measure.
      if (body.includes('id="root"') && body.includes('/assets/')) {
        // One extra request for the wasm artifact's MIME is not needed here: the
        // page load itself fails loudly if `application/wasm` is wrong.
        return;
      }
      lastError = 'the served index.html is not the built chrome bundle';
    } catch (error) {
      lastError = error.message;
    }
    await sleep(150);
  }
  throw new Error(`the static server never became ready at ${url}: ${lastError}`);
}

/** One non-200-aware GET returning the body as text. */
function httpGet(url, timeout) {
  return new Promise((resolve, reject) => {
    const request = http.get(url, { timeout }, response => {
      if (response.statusCode !== 200) {
        response.resume();
        reject(new Error(`GET ${url} -> ${response.statusCode}`));
        return;
      }
      let body = '';
      response.setEncoding('utf8');
      response.on('data', chunk => { body += chunk; });
      response.on('end', () => resolve(body));
    });
    request.on('timeout', () => request.destroy(new Error('request timed out')));
    request.on('error', reject);
  });
}

function sleep(ms) {
  return new Promise(resolve => { setTimeout(resolve, ms); });
}

/** Resolve `true` once `child` exits, `false` after `timeout` ms. */
function waitForExit(child, timeout) {
  if (child.exitCode !== null) return Promise.resolve(true);
  return new Promise(resolve => {
    const timer = setTimeout(() => resolve(false), timeout);
    child.once('exit', () => {
      clearTimeout(timer);
      resolve(true);
    });
  });
}

/** SIGTERM, then SIGKILL: the server must not outlive the run. */
async function stopServer(child) {
  if (!child || child.exitCode !== null) return;
  child.kill('SIGTERM');
  if (await waitForExit(child, 3000)) return;
  child.kill('SIGKILL');
  await waitForExit(child, 2000);
}

module.exports = { waitForServer, httpGet, sleep, waitForExit, stopServer };
