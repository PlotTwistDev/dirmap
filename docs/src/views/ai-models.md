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

Click a model to see everything known about it on the right, with the full path of each of its
files. Double-click it, or right-click and choose **Show in Files**, to open the folder it's in
with the file selected. **Copy path** copies the path of its first file.

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
