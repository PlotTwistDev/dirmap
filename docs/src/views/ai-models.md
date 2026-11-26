# The AI Models workspace

The **✨ AI Models** workspace (**Ctrl+4**, **⌘4** on a Mac) lists the AI models on this
computer: language models saved by **Ollama**, **LM Studio** and **Hugging Face**, and model files
saved anywhere else, such as Stable Diffusion or ComfyUI checkpoints. Models are often the largest
files on a disk, and the same model is easily downloaded twice by different apps.

## Where Filescape looks

Filescape looks for models as soon as the workspace opens. The panel on the left lists where it
looked:

| Where | What Filescape shows |
|---|---|
| Ollama (`~/.ollama/models`, the folder `OLLAMA_MODELS` points to, and on Linux `/usr/share/ollama/.ollama/models`) | Each installed model by its Ollama name, such as `llama3.2:3b`, with its vision add-on if it has one. Ollama names its files by checksum; Filescape reads Ollama's own list to name them. |
| LM Studio (`~/.lmstudio/models`, or the older `~/.cache/lm-studio/models`) | Each model file. |
| Hugging Face (`~/.cache/huggingface/hub`, or the folder `HF_HUB_CACHE` or `HF_HOME` points to) | Each downloaded model by its Hugging Face name, such as `google/gemma-3-1b-it`. A repository with several GGUF versions shows each one. |
| Downloads | Model files you downloaded by hand. |
| Folders you add | Press **✚ Add folder…** to also look in a folder of your choice. Press **✖** beside it to stop. |
| The Storage Analysis scan | If you've scanned a drive or folder in Storage Analysis, its model files are included without scanning again. |

Outside the Ollama, LM Studio and Hugging Face folders, a file counts as a model when it is 16 MB or
more and has a model extension: `.gguf`, `.safetensors`, `.ckpt`, `.pt`, `.pth`, `.onnx`,
`.mlmodel` and the others in the [AI models category](../filtering/categories.md), or is named
`pytorch_model*.bin` or `model*.bin`.

Filescape only reads the first few kilobytes of each model file, where the model describes itself,
so looking is quick. **⟳ Look again** looks once more, for example after downloading a model.
**Cancel** stops it.

Files that belong to one model are shown as one entry: numbered parts
(`model-00001-of-00004.safetensors`), all the generic files of one folder (`model.safetensors`,
`pytorch_model.bin`), and the parts of an Ollama model.

## Reading the list

Models are sorted by size, largest first. Each one shows its name, size and where it's from, then
a line of details:

`GGUF · qwen3 · 8B · Q4_K_M · 40K context`

| Detail | Meaning |
|---|---|
| Format | **GGUF** (used by Ollama, LM Studio and llama.cpp), **safetensors**, **PyTorch**, **ONNX** or **Core ML**. |
| Family | The model's architecture, such as llama, qwen3 or gemma3, from a GGUF file. |
| Parameters | The number of parameters, such as 8B (8 billion). Larger models know more and need more memory. |
| Precision | How the numbers in the model are stored. **F16**, **BF16** and **F32** are full precision; names like **Q4_K_M** or **Q8_0** are smaller, compressed (quantized) versions. |
| Context | The longest text the model can read at once, in tokens (roughly ¾ of a word each). |
| Add-on | A vision projector or adapter that works together with a model, not on its own. |

Click a model to see what's known about it on the right, with the full path of each of its files.
Right-click it and choose **Show in Files**, or press **Show in Files** on the right, to open the
folder it's in with the file selected. **Copy path** copies the path of its first file.

## Inspecting a model

Double-click a model, or press **🔍 Inspect**, to see what's inside it. Filescape reads only the
part of each file where the model describes itself and lists its pieces, never the weights, so
this takes a moment even for a model of many gigabytes. **⏴ All models** or **Esc** goes back to the
list.

**Overview** lists what's known about the model, such as its architecture, layers, attention
heads, context length, vocabulary, licence and number of parameters. A tag beside each fact says
where it comes from:

| Tag | Meaning |
|---|---|
| **file** | Read from the model file itself. |
| **calculated** | Worked out by Filescape from the model file, for example the number of parameters, by adding up its tensors. |
| **publisher** | Stated in the publisher's files beside the model (`config.json` or the README of a Hugging Face download). Filescape hasn't checked it against the weights. |

Below, **How the numbers are stored** shows how much of the model is kept in each number type. A
**Q4_K_M** model, for example, keeps most of its numbers as Q4_K and a few important ones at the
higher Q6_K precision.

**Health** checks the model and lists what it found, problems first:

- whether every part of a model split into several files is there;
- whether a file is cut short, as a stopped download leaves it, or damaged, with tensors that
  overlap or are out of place;
- whether the format is safe to open (GGUF and safetensors) or can contain code (PyTorch
  checkpoints);
- whether the number of parameters matches the model's size label, and whether a GGUF model has a
  vocabulary and a chat template.

These checks only use what was already read, so they're instant. Ollama and Hugging Face name each
downloaded file by its SHA-256 checksum. For those, **Verify checksum** reads the whole model and
compares, which proves not a single byte has changed since it was downloaded. It takes a while for
a large model; **Cancel** stops it.

**Check the numbers** reads a small sample of every tensor (about 128 KB of each) and looks for
broken values (NaN or infinite, which make a model give wrong answers) and tensors that are all
zeros, which usually means their data was damaged or left out. Click a tensor it lists to see it
in **Tensors**.

**Will it run?** estimates how much memory the model needs on this computer, and where it would
run:

- **This computer** shows its memory and its graphics card's memory. On a Mac with Apple silicon
  the two are shared. If Filescape can't find the graphics card's memory, type it in.
- Move **Context** to choose how much text the model reads at once, in tokens (about ¾ of a word
  each). The memory a model needs grows with it, because it keeps a cache of everything it has
  read. **Compact (Q8)** stores that cache in about half the space.
- The result says whether the model runs fully on the graphics chip (fast), split between the
  graphics card and main memory, in main memory on the processor (slow), or not at all. The bar
  shows the model itself, the cache and working space, against the memory available.
- **At other context lengths** lists the same for each length, and **Smaller versions of this
  model** what a compressed version would need, such as Q4_K_M.

These are estimates for apps based on llama.cpp, such as Ollama and LM Studio. Each app adds a
little of its own.

**Structure** shows how the model is built and where its space goes:

- The strip at the top shows the model's layers in order, left to right. A taller column takes
  more space, which shows at a glance which layers were kept at a higher precision.
- Below, the model is drawn as a map, like the [treemap](treemap.md) of a drive: each box is a
  layer, the embeddings or the output, sized by the space it takes and divided into its tensors.
  Click a box, or a column of the strip, to see that layer on its own, with each tensor named.
  **⏴ Whole model**, or a right-click on the map, goes back. Click a tensor to find it in the
  Tensors list.
- **Colour by** switches between **What it does** (embeddings, attention, feed-forward,
  mixture-of-experts, normalisation and output) and **Number type** (such as Q4_K or Q6_K).
  Rest the pointer on a colour in the key to see what it means.

**Tensors** lists every tensor (block of numbers) in the model with its type, shape, number of
values and size. Type in the box to show only some, such as `attn` for the attention tensors or
`Q6_K` for one type. Click a column heading to sort by it, and again to reverse. Right-click a
tensor to copy its name.

Click a tensor to see more about it on the right, with its numbers: their average, spread,
smallest and largest, how many are zero, and a histogram of their values. For a large tensor
Filescape reads a sample from across it, which takes a moment. It reads full and half precision
(F32, F16, BF16) and the common compressed types (Q4_0, Q4_1, Q5_0, Q5_1, Q8_0, Q4_K, Q5_K and
Q6_K); for other types only the facts are shown.

**Tokens** shows the model's vocabulary: the pieces of text, called tokens, that it reads and
writes. Filescape reads it from a GGUF file, or from `tokenizer.json` beside a Hugging Face model.

- The summary gives the number of tokens, the kind of tokenizer, how many tokens start a new word,
  and the special tokens that mark the start and end of text and of each turn in a conversation.
- **What the tokens are** sorts every token by its first letter: Latin, Chinese and Japanese,
  Cyrillic, Arabic and so on, numbers, punctuation and indentation. It's a quick guide to which
  languages a model handles well, since a language with few tokens takes many more of them to
  write, which is slower and fills the context sooner. Click a line to list only those tokens.
- **How long the tokens are** is a histogram of their lengths in characters, and **Longest**
  shows the longest ones.
- **Chat template** shows how a conversation is laid out for the model, and says which format it
  uses (such as ChatML or Llama 3) and whether it mentions tool calls, a thinking mode or images.
- On the right, every token is listed with its number. A space is shown as `·`, a new line as `↵`
  and a tab as `⇥`. Type in the box to find tokens, or `#` and a number to find one by its number.
  Right-click a token to copy it. Letters the app's fonts can't show, such as Chinese, are shown
  by their code points (`U+4E2D`); copy the token to see it elsewhere.

Some tokens are only part of a character: a Chinese character or an emoji takes several bytes,
and a model joins these pieces to spell anything its vocabulary lacks.

**Metadata** lists every entry the model file carries, such as its training settings and
tokenizer, with a box to find one.

GGUF and safetensors models can be inspected. PyTorch checkpoints (`.ckpt`, `.pt`, `.pth` and
`.bin`) are never opened, because they are saved with Python's pickle, which can run code when a
file is opened. Their names and sizes are shown, and the same model in safetensors or GGUF format
can be inspected instead. Files only in the cloud aren't read, so as not to download them.

## Copies and unused downloads

The heading says how much space is used by **extra copies** and by **Ollama downloads no model
uses**.

- **copy** marks a model whose files are byte-for-byte the same as a model higher in the list,
  for example the same GGUF file in LM Studio and in Downloads. **has copies** marks the first
  one. The details on the right list where the other copies are. Copies are confirmed by reading
  the files, as the [Duplicates](../analysis/duplicates.md#how-detection-works) tab does, but only
  files of exactly the same size are read.
- **not used by any Ollama model** marks an Ollama file that none of Ollama's installed models
  needs, usually left behind by a removed model or a stopped download.

Filescape doesn't delete anything by itself. To free space, delete a copy in the Files workspace
as you would any file. For Ollama, removing a model with `ollama rm` is the safest way, since
Ollama keeps its own list of the files each model uses.

Files that are only in the cloud are listed but not read, so their details are missing and they
aren't checked for copies. Reading them would download them.

## On the command line

`filescape --cli --models PATH` prints the models in a folder without opening a window. See
[Command line](../reference/command-line.md).
