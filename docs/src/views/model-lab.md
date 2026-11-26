# The Lab

The **Lab** shows how an AI model writes: what it would write next, how sure it is, and what else it
considered. It runs the model on this computer, like [Ask AI](ask-ai.md), and changes nothing on
disk. The Lab is free.

Open the [AI Models workspace](ai-models.md) (**Ctrl+4**, **⌘4** on a Mac) and press **🔬 Lab** in
the toolbar. The models on the left are the ones Ask AI lists; load one the same way. The Lab and
Ask AI share the loaded model, so switching between them doesn't load it again.

## Seeing what comes next

Type some text, such as "The capital of France is", and press **Next token**. A model writes in
*tokens*: whole words, parts of words, spaces or punctuation. The Lab shows the token it picked and
the ten tokens the model thought likeliest:

- **Model** is the model's own probability for each token.
- **After settings** is what's left once the sampling settings (below) have had their say. A dash
  means a setting ruled the token out.
- The grey bar is the model's probability; the coloured bar is the one after the settings.

**Use this** writes a different token in its place and shows what the model would write after it,
so you can follow a road the model didn't take. **Start again** forgets the tokens you've chosen.

**Write 30** (or 10, 100 or 300) writes several tokens at once. Each is coloured by how sure the
model was of it: green for confident, amber, then red for unlikely. Click a token to see its
alternatives. **Keep these** adds what was written to the text, to carry on from there. The line
under the text gives the average confidence and the speed.

Tick **As a chat message** to send the text the way Ask AI sends a question, wrapped in the model's
chat template. You then see how the model starts an answer rather than how it carries on a
sentence. Some models think before they answer; you'll see that too.

## The sampling settings

These decide which token is picked from the model's probabilities, in this order:

| Setting | What it does |
|---|---|
| **Top-k** | Only the k likeliest tokens can be picked. 0 turns it off. |
| **Top-p** | Only the likeliest tokens that together make up this share (0.95 is 95%) can be picked. 1 turns it off. |
| **Min-p** | Tokens less likely than this share of the likeliest one are ruled out. 0 turns it off. |
| **Temperature** | How adventurous the pick is among what's left. 0 always picks the likeliest. 1 picks as often as the model says. Higher gives unlikely tokens more of a chance. |
| **Seed** | The same seed and settings pick the same tokens again. **🎲** chooses another. |

Change a setting after **Next token** and the Lab works out the next token again straight away, so
you can watch the effect. **Ask AI's settings** puts back the ones Ask AI uses (temperature 0.6,
top-k 40, top-p 0.95, min-p 0.05).

The Lab works out these probabilities itself, so what it shows is exactly what it picks from.

## What to expect

A model's probabilities say how sure it is of the next token, not whether it's right: a model can
be confidently wrong. Small models are less sure and wander more. The Lab only shows text; it never
acts on what a model writes.
