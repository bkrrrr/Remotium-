// argv.json handling without editor dependencies, so it can be unit tested.
import * as os from 'os';
import * as path from 'path';
import { applyEdits, modify, parse } from 'jsonc-parser';

export const EXTENSION_ID = 'remotium.client';

/** Where VSCodium reads its runtime arguments (same rule as the editor itself). */
export function argvPath(dataFolderName: string, env: NodeJS.ProcessEnv = process.env, home = os.homedir()): string {
	return env.VSCODE_PORTABLE ? path.join(env.VSCODE_PORTABLE, 'argv.json') : path.join(home, dataFolderName, 'argv.json');
}

export function argvHasProposal(text: string): boolean {
	const v = parse(text, [], { allowTrailingComma: true })?.['enable-proposed-api'];
	return Array.isArray(v) && v.includes(EXTENSION_ID);
}

/** Add the extension to `enable-proposed-api`, keeping comments and other entries. */
export function addProposal(text: string): string {
	if (argvHasProposal(text)) {
		return text;
	}
	const base = text.trim() ? text : '{\n}\n';
	const current = parse(base, [], { allowTrailingComma: true })?.['enable-proposed-api'];
	const list = Array.isArray(current) ? [...current, EXTENSION_ID] : [EXTENSION_ID];
	return applyEdits(base, modify(base, ['enable-proposed-api'], list, { formattingOptions: { insertSpaces: false, tabSize: 4, eol: '\n' } }));
}
