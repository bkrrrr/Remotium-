import { strict as assert } from 'assert';
import { test } from 'node:test';
import { isFingerprint, normalizeFingerprint, parsePairingLink, toWebSocketUrl } from '../../src/pairing';

const code = 'rmp_KXL5Z7Hc1hbs_zd_zKiCpY_tz0kzJ1yS_3Ofw-kLmL4';
const fp = 'cdc841f4144d89adbc7867264cc6e18997ffe544029e2ff86f36e55d7bb5f6dd';

test('VSCodium link with fingerprint', () => {
	const l = parsePairingLink(`vscodium://remotium.client/pair?host=https%3A%2F%2Fdevbox%3A8443&code=${code}&fp=${fp}`);
	assert.deepEqual(l, { url: 'https://devbox:8443', code, fingerprint: fp });
});

test('browser link (publicly trusted certificate)', () => {
	assert.deepEqual(parsePairingLink(`https://dev.example.com/pair?code=${code}`), { url: 'https://dev.example.com', code, fingerprint: undefined });
	assert.equal(parsePairingLink(`https://h:1/base/pair?code=${code}`).url, 'https://h:1/base');
});

test('rejects broken links', () => {
	assert.throws(() => parsePairingLink('hello'));
	assert.throws(() => parsePairingLink(`https://h/other?code=${code}`));
	assert.throws(() => parsePairingLink('https://h/pair?code=short'));
	assert.throws(() => parsePairingLink(`vscodium://remotium.client/pair?host=ftp%3A%2F%2Fh&code=${code}`));
	assert.throws(() => parsePairingLink(`vscodium://remotium.client/pair?host=https%3A%2F%2Fh&code=${code}&fp=abc`));
});

test('fingerprints in either notation', () => {
	const pretty = fp.match(/../g)!.join(':').toUpperCase();
	assert.equal(normalizeFingerprint(pretty), fp);
	assert.ok(isFingerprint(pretty));
	assert.equal(toWebSocketUrl('https://h:8443'), 'wss://h:8443');
	assert.equal(toWebSocketUrl('http://127.0.0.1:9'), 'ws://127.0.0.1:9');
});
