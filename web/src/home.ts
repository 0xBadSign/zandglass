import './style.css';
import { h, hourglass } from './lib/dom';
import { mountShell } from './lib/shell';

const main = mountShell('home');
main.append(
  h(
    'section',
    { class: 'hero' },
    hourglass(0.45),
    h('h1', {}, 'Sandglass'),
    h(
      'p',
      { class: 'lead' },
      'Time-locked inheritance and scheduled payments for shielded Zcash. ' +
        'The full site is coming soon. The two tools below already work.',
    ),
  ),
  h(
    'section',
    { class: 'choices' },
    h(
      'div',
      { class: 'card' },
      h('h2', {}, 'Become an heir'),
      h(
        'p',
        {},
        'Someone wants to name you as their heir? Create your heir key here, in your browser, and send them your heir code.',
      ),
      h('a', { class: 'btn-primary', href: './heir.html' }, 'Create my heir key'),
    ),
    h(
      'div',
      { class: 'card' },
      h('h2', {}, 'Claim an inheritance'),
      h(
        'p',
        {},
        'Got a claim link or a kit file? Open it here once the time lock has passed, with your heir phrase.',
      ),
      h('a', { class: 'btn', href: './claim.html' }, 'Open a kit'),
    ),
  ),
);
