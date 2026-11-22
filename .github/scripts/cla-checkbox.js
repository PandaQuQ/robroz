'use strict';

/**
 * Shared CLA checkbox helpers used by .github/workflows/cla.yml
 * and .github/scripts/cla-checkbox.test.js.
 *
 * The copyright holder and PRs opened from the PandaQuQ account do not
 * need to sign; bot PRs are also skipped. External contributors must
 * tick the CLA checkbox.
 */

const COPYRIGHT_HOLDER_LOGINS = new Set(['pandaquq']);

function shouldSkipClaCheck(user) {
  if (!user) return false;
  const login = String(user.login || '');
  const type = String(user.type || '');
  if (COPYRIGHT_HOLDER_LOGINS.has(login.toLowerCase())) return true;
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

module.exports = { shouldSkipClaCheck, claCheckboxTicked, COPYRIGHT_HOLDER_LOGINS };
