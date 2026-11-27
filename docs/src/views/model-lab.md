# The Lab

The **Lab** shows how an AI model writes: what it would write next, how sure it is, and what else it
considered. It runs the model on this computer, like [Ask AI](ask-ai.md), and changes nothing on
disk. The Lab is free.

Open the [AI Models workspace](ai-models.md) (**Ctrl+4**, **⌘4** on a Mac) and press **🔬 Lab** in
the toolbar. The models on the left are the ones Ask AI lists; load one the same way. The Lab and
Ask AI share the loaded model, so switching between them doesn't load it again.

Each tab has an **About** section under its name that explains what it shows and what it
doesn't: a probability is how likely the model thinks a token is, not whether it's true.

## Seeing what comes next

Type some text, such as "The capital of France is", and press **Show next token**. A model writes in
*tokens*: whole words, parts of words, spaces or punctuation. The Lab shows the token it picked and
the ten tokens the model thought likeliest:

- **Model** is the model's own probability for each token.
- **After settings** is what's left once the sampling settings (below) have had their say. A dash
  means a setting ruled the token out.
- The grey bar is the model's probability; the coloured bar is the one after the settings.

**Use this** writes a different token in its place and shows what the model would write after it,
so you can follow a road the model didn't take. **⏴ Step back** takes off the last token you chose
and shows the choices there again, so you can try another road. **Start again** forgets the tokens
you've chosen.

Beside the alternatives, **uncertainty** says how spread out the model's guesses were, in bits,
before the settings: 0 bits means it was certain, 1 bit is as unsure as a coin toss, and n bits is
as unsure as a fair pick among 2ⁿ tokens ("like a fair pick among 169 tokens"). It measures how
sure the model was, not whether the text is true.

**Write 30 tokens** (or 10, 100 or 300) writes several tokens at once. Each is coloured by how sure the
model was of it: green for confident, amber, then red for unlikely. Click a token to see its
alternatives. **Keep these** adds what was written to the text, to carry on from there. The line
under the text gives the average confidence, the average uncertainty and the speed.

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

Change a setting after **Show next token** and the Lab works out the next token again straight away, so
you can watch the effect. **Ask AI's settings** puts back the ones Ask AI uses (temperature 0.6,
top-k 40, top-p 0.95, min-p 0.05).

The Lab works out these probabilities itself, so what it shows is exactly what it picks from.

## Looking inside the model

A model reads text in *layers*, one after another: a small model has a dozen or so, a large one
eighty or more. Each layer adjusts every token's meaning a little, using the tokens before it, and
the last layer's result decides what comes next. Press **Inside** at the top of the Lab, type some
text and press **Look inside** to watch this happen. The Lab reads up to 256 tokens at a time.

The map shows a column for each token and a row for each layer, the first at the top. The brighter
a square, the more that layer changed the token: a small nudge is dark, a big rewrite is bright
orange; the scale under the map shows the range, from a change of 2% of the token's length or
less to 200%. Hover over a square for the exact amount. The **average** column on the right shows
how much each layer changes the tokens on average; hover over it for the figure. The first row stays dark, because it is where
each token's meaning starts. Look for the layers where many tokens change at once, and for tokens
that keep changing late (often the ones the next word depends on).

Click a token to see **what each layer would predict** after it. The Lab takes that layer's result
and finishes it the way the model's last layer does, so you see what the model would write if it
stopped there, with the five likeliest tokens and their probabilities. Early layers often guess
something vague, such as a space or "the"; the right answer tends to appear partway through and
grow more certain. The line above the table says from which layer the final answer stays on top.
Researchers call this a *logit lens*.

The last layer's prediction is the model's own: it matches what **Next token** shows. Looking
inside reads the model's file to work out the predictions and changes nothing in it.

## Steering a model

Press **Steer** at the top of the Lab to push a model towards one idea and away from another while
it writes. Type the text for it to carry on, a word or phrase in **Towards** (such as "Love") and
one in **Away from** (such as "Hate"), and press **Write both ways**. The Lab writes the text twice
with the same settings and seed: once as the model would, and once pushed. The two appear side by
side, each token coloured by how sure the model was.

The push is worked out from your two words alone. The Lab reads each one, takes what the chosen
layer makes of it, and adds the difference to that layer's output for every token while it writes.
Researchers call this *activation steering*, and the difference a *steering vector*.

- **Strength** is how hard to push. Below 0 pushes the other way. Too strong and the text stops
  making sense; the line under the result says how big the push was beside that layer's own
  output.
- **At layer** chooses where to push. Early layers change words more than ideas, and late ones
  have little room left to change anything; about a third of the way in usually works best and is
  where the Lab starts.
- Leave one box empty to push towards (or away from) a single idea.
- The pairs under the boxes, such as **Happy / Sad** or **Formal / Casual**, fill both boxes in one
  click.
- **Sweep strengths** writes the text without a push and then at strengths 2, 4, 6, 8 and 10, one
  under another, so you can see where the push starts to show and where the text falls apart.
  **■ Stop** stops it after the one it's writing.

The push only exists while the text is written, in a copy of the model's working memory that's
thrown away afterwards. The model's file, Ask AI and the other Lab pages aren't affected. The
sampling settings are the ones on **Next token**.

## Comparing settings or models

Press **Compare** at the top of the Lab to write the same text twice, side by side, with two sets
of sampling settings. Each side has its own Temperature, Top-k, Top-p and Min-p, with three
starting points: **Likeliest** (temperature 0), **Ask AI's** and **Adventurous** (temperature 1.2,
no limits). Press **Write 30 tokens both ways** (or 10 or 100); both sides use the seed set on **Next token**, so only the
settings differ. Under each text are its average confidence, average uncertainty and speed, and
the line under both says where the two texts part.

To compare two models, write once, tick **Keep this result** on one side, load another model on
the left and press **Write 30 tokens** again: only the other side is written, with the new model. Each side
names the model that wrote it. If you change the text after keeping a side, that side says it was
written from different text.

One run shows what one seed happened to pick. Above temperature 0, try a few seeds before deciding
which settings or model you prefer. For speeds, the **Speed** page measures more carefully.

## Testing a model's speed

Press **Speed** at the top of the Lab to see how fast the loaded model runs on this computer.
Choose how much text it reads first (**Read**), how much it writes (**Write**) and how many times
to run the test (**Runs**: the middle result is kept, so one slow run doesn't count), then press
**Run speed test**. **■ Stop** stops it.

The result shows:

- **Tokens a second written**: how fast answers appear. A token is about ¾ of a word, and people
  read about 4 words a second, so 6 tokens a second or more keeps ahead of you.
- **Tokens a second read**: how fast the model takes in a question or a document before it
  answers. This is usually much faster than writing.
- **To the first word**: how long you'd wait for an answer to start after sending that much text.

Once a model has been tested more than once, a chart under the result shows its last tests (up to
24) as bars of tokens a second written; hover over a bar to see that test's settings.

Every result is kept, with the model, where it ran (the graphics chip, or the processor and how
many threads), the context length and the date, so you can compare models and settings in the
**All results** table; hover over **Where** to see the processor or graphics chip by name. The
results are kept in `lab-speed.json` in Filescape's settings folder. **Export…** saves them all as a
CSV file for a spreadsheet, with every setting and the hardware each test ran with.

Other busy apps slow the test down, and the test slows them down while it runs.

## What to expect

A model's probabilities say how sure it is of the next token, not whether it's right: a model can
be confidently wrong. Small models are less sure and wander more. The Lab only shows text; it never
acts on what a model writes.
