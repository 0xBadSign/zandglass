import { h, hourglass } from './dom';

export type PageId = 'home' | 'playground' | 'heir' | 'claim';

/** Build the shared header and footer; returns the <main> element to fill. */
export function mountShell(current: PageId, opts: { wide?: boolean } = {}): HTMLElement {
  const root = document.getElementById('root');
  if (!root) throw new Error('missing #root');
  if (opts.wide) document.body.classList.add('wide');
  const link = (id: PageId, href: string, text: string) =>
    h('a', { href, 'aria-current': current === id ? 'page' : null }, text);
  const main = h('main', { id: 'main' });
  root.replaceChildren(
    h('a', { class: 'skip', href: '#main' }, 'Skip to content'),
    h(
      'header',
      { class: 'site-header' },
      h('a', { class: 'brand', href: './' }, hourglass(0.4), 'Sandglass'),
      h(
        'nav',
        { class: 'site-nav', 'aria-label': 'Main' },
        link('playground', './playground', 'Playground'),
        link('heir', './heir', 'Become an heir'),
        link('claim', './claim', 'Claim'),
      ),
    ),
    main,
    h(
      'footer',
      { class: 'site-footer' },
      h(
        'p',
        {},
        'Everything on this page runs in your browser. Your words and your keys are never sent anywhere. ' +
          'The only network requests are to the drand time servers and, for a claim link or the playground, to the keeper that holds the kit.',
      ),
    ),
  );
  return main;
}
