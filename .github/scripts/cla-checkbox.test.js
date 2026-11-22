'use strict';

const assert = require('node:assert/strict');
const { claCheckboxTicked, shouldSkipClaCheck } = require('./cla-checkbox.js');

const template =
  '- [ ] I have read and agree to the [Contributor License Agreement](https://github.com/PandaQuQ/robroz/blob/main/CLA.md).';

const ticked = template.replace('- [ ]', '- [x]');

assert.equal(claCheckboxTicked(''), false, 'empty body');
assert.equal(claCheckboxTicked(null), false, 'null body');
assert.equal(claCheckboxTicked(template), false, 'unticked checkbox');
assert.equal(claCheckboxTicked(ticked), true, 'ticked lowercase [x]');
assert.equal(
  claCheckboxTicked(ticked.replace('[x]', '[X]')),
  true,
  'ticked uppercase [X]',
);
assert.equal(
  claCheckboxTicked('*\t[x] I have read and agree to the Contributor License Agreement.'),
  true,
  'asterisk list marker',
);
assert.equal(
  claCheckboxTicked('1. [x] I have read and agree to the Contributor License Agreement.'),
  true,
  'ordered list',
);
assert.equal(
  claCheckboxTicked('  - [X] I have read and agree to the Contributor License Agreement.\n'),
  true,
  'leading space and trailing newline',
);
assert.equal(
  claCheckboxTicked('- [x] I agree to something else\n' + template),
  false,
  'other box ticked, CLA still unticked',
);
assert.equal(
  claCheckboxTicked(template + '\n- [x] I have read and agree to the Contributor License Agreement.'),
  true,
  'CLA ticked among other boxes',
);
assert.equal(
  claCheckboxTicked('- [x] no mention of the agreement'),
  false,
  'ticked box without CLA text',
);

assert.equal(shouldSkipClaCheck({ login: 'dependabot[bot]', type: 'Bot' }), true);
assert.equal(shouldSkipClaCheck({ login: 'github-actions[bot]', type: 'Bot' }), true);
assert.equal(shouldSkipClaCheck({ login: 'dependabot', type: 'User' }), true);
assert.equal(shouldSkipClaCheck({ login: 'PandaQuQ', type: 'User' }), true, 'copyright holder is exempt');
assert.equal(shouldSkipClaCheck({ login: 'pandaquq', type: 'User' }), true, 'login match is case-insensitive');
assert.equal(shouldSkipClaCheck({ login: 'someone-else', type: 'User' }), false);
assert.equal(
  shouldSkipClaCheck({ login: 'someone-else', type: 'User' }) || claCheckboxTicked(''),
  false,
  'external author with empty body must fail',
);

console.log('cla-checkbox tests passed');
