import { strict as assert } from 'assert';
import { test } from 'node:test';
import { parse } from 'jsonc-parser';
import { addProposal, argvHasProposal, argvPath, EXTENSION_ID } from '../../src/argv';

test('creates the entry in an empty or missing file', () => {
	const out = addProposal('');
	assert.deepEqual(parse(out)['enable-proposed-api'], [EXTENSION_ID]);
});

test('keeps comments and other settings of the default argv.json', () => {
	const text = `// This configuration file allows you to pass permanent command line arguments to VS Code.
{
	// Use software rendering instead of hardware accelerated rendering.
	// "disable-hardware-acceleration": true,

	// Unique id used for correlating crash reports sent from this instance.
	"crash-reporter-id": "abc"
}
`;
	const out = addProposal(text);
	assert.ok(out.includes('// Use software rendering'), 'comments kept');
	assert.equal(parse(out)['crash-reporter-id'], 'abc');
	assert.deepEqual(parse(out)['enable-proposed-api'], [EXTENSION_ID]);
	assert.ok(argvHasProposal(out));
});

test('appends to an existing list and is idempotent', () => {
	const text = '{\n\t"enable-proposed-api": ["jeanp413.open-remote-ssh"], // ssh\n}\n';
	const out = addProposal(text);
	assert.deepEqual(parse(out, [], { allowTrailingComma: true })['enable-proposed-api'], ['jeanp413.open-remote-ssh', EXTENSION_ID]);
	assert.ok(out.includes('// ssh'));
	assert.equal(addProposal(out), out);
});

test('argv.json location follows VSCodium (portable mode, home)', () => {
	assert.equal(argvPath('.vscode-oss', {}, '/home/u'), '/home/u/.vscode-oss/argv.json');
	assert.equal(argvPath('.vscode-oss', { VSCODE_PORTABLE: '/p' }, '/home/u'), '/p/argv.json');
});
