import { roundAt } from './drand';

const app = document.getElementById('app');
if (app) {
  const note = document.createElement('p');
  note.textContent = `Current drand quicknet round: ${roundAt(Date.now() / 1000)}`;
  app.append(note);
}
