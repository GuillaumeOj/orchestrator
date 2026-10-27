import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import HostProbe from './fixtures/HostProbe.svelte';

describe('TermHost', () => {
  it("binds the host element before the pane's onMount runs", () => {
    /** @type {HTMLElement | undefined} */
    let got;
    render(HostProbe, { seen: (h) => (got = h) });
    expect(got).toBeInstanceOf(HTMLElement);
    expect(got?.className).toContain('host');
  });
});
