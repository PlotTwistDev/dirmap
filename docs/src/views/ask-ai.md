# Ask AI

**Ask AI** lets you chat with an AI model that's already on this computer, run by Filescape
itself. You don't need Ollama, LM Studio or an internet connection, and nothing you ask leaves the
computer. Ask AI is free.

Open the [AI Models workspace](ai-models.md) (**Ctrl+4**, **⌘4** on a Mac) and press **💬 Ask AI**
in the toolbar. Press **Library** to go back to the list of models. You can also select a model
in the library and press **💬 Ask AI** beside **🔍 Inspect**.

## Choosing a model

The left side lists the models Filescape can run: GGUF models, the kind Ollama and LM Studio
download, and `.gguf` files from Hugging Face or elsewhere. The smallest come first, since they
load and answer quickest. Models in other formats, and copies of a model listed earlier, aren't
listed.

If there's no model yet, see [Getting a small model](#getting-a-small-model) below.

Click a model and press **Load**, or double-click it. Filescape first checks the model fits in this
computer's memory, and says so if it doesn't. Loading takes from a few seconds to a minute,
depending on the model's size and the disk. **Unload** frees the memory again.

- **Context** is how much of the conversation the model keeps in mind, in tokens (about ¾ of a
  word each). A longer context needs more memory. It takes effect the next time a model is loaded.
  When a conversation grows longer than the context, the oldest messages are left out.
- **Creativity** sets how varied the answers are. Low picks the likeliest words, so the same
  question gets the same answer; high is more varied, and more likely to wander.

On a Mac with Apple silicon, models run on the graphics chip. On Windows and Linux they run on the
processor, which needs one with AVX2 (nearly every PC made since 2015). A small model of 1 to 4
billion parameters answers at a comfortable reading speed on most computers; larger ones are
slower.

## Getting a small model

If you don't have a model, press **Get a small model…** on the left. (When you already have
models, it's a smaller button under the list, until you've downloaded it.) Filescape shows what it
would download before anything happens:

- **Qwen3 1.7B**, made by Qwen (Alibaba Cloud): a small general-purpose model that answers in many
  languages and can think before it answers.
- A 1.7 GB download, which needs about 2.2 GB of memory to run.
- Its licence, Apache 2.0, and the publisher's page on Hugging Face, both of which you can open.
- The folder it's saved in: `%LOCALAPPDATA%\Filescape\Models` on Windows,
  `~/Library/Application Support/Filescape/Models` on a Mac and `~/.local/share/filescape/models`
  on Linux.

Press **Download** to get it, or **Not now**. The file comes from Qwen's own page on Hugging Face,
at a fixed version, and Filescape checks its SHA-256 checksum before using it; a file that doesn't
match is removed. When it's done, Filescape lists it and loads it, ready for your first question.

**Stop** pauses the download. What has downloaded is kept, and **Carry on downloading** continues
from there, even after Filescape is closed and opened again. If there isn't enough free space,
Filescape says so before it starts.

This is the only time Ask AI uses the internet. Nothing about you or your files is sent. To remove
the model, delete its file from the folder above.

## Asking

Type a question and press **Enter**, or **Send**. **Shift+Enter** starts a new line. The answer
appears as it's written; **■ Stop** ends it early. **New chat** starts a new conversation with
the same model, and **Copy** under an answer copies it.

Some models, such as Qwen 3 and DeepSeek R1, think before they answer. Their thinking is folded
away under **Thought about it**; click it to read it.

Under the last answer, Filescape shows how many tokens it took and how fast they were written.

## What to expect

Answers come from the model, not from Filescape, and can be wrong, especially from small models.
Filescape only shows them: it never deletes, moves or opens anything because of what a model
wrote, and links in an answer aren't opened.
