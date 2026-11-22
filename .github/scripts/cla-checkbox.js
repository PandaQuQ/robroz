'use strict';

/**
 * Shared CLA checkbox helpers used by .github/workflows/cla.yml
 * and .github/scripts/cla-checkbox.test.js.
 */

function shouldSkipClaCheck(user) {
  if (!user) return false;
  const login = String(user.login || '');
  const type = String(user.type || '');
  if (type.toLowerCase() === 'bot') return true;
  return /\[bot\]$/i.test(login) || login.toLowerCase() === 'dependabot';
}

/**
 * True when the PR body has a ticked markdown checkbox on a line that
 * refers to the Contributor License Agreement. `[x]` is case-insensitive.
 * Unticked boxes (`[ ]`) do not match.
 */
function claCheckboxTicked(body) {
  if (!body) return false;
  return /^\s*(?:[-*]|\d+\.)\s*\[\s*x\s*\][^\n]*contributor license agreement/im.test(
    String(body),
  );
}

module.exports = { shouldSkipClaCheck, claCheckboxTicked };
