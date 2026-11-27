//! Plain explanations of AI terms, for tooltips and information icons in the AI Models
//! workspace. One sentence or two each, and accurate: these are read by people learning.

/// The explanation of `term` (a fact label or a concept), or "" when there isn't one.
pub fn glossary(term: &str) -> &'static str {
    match term {
        "Parameters" | "Size label" => {
            "The numbers (weights) the model learned in training. More parameters usually means a more capable model that needs more memory and runs slower."
        }
        "Weights" => "The learned numbers that make up a model. A model file is mostly its weights.",
        "Tokens" | "Token" => {
            "The pieces a model reads and writes: whole words, parts of words, punctuation or single bytes. In English a token is about three quarters of a word."
        }
        "Tokenizer" | "Tokenisation" => "How text is split into tokens before the model sees it, and joined back after. Each model family has its own.",
        "Vocabulary" => "Every token the model knows. It reads and writes text only as these tokens.",
        "Context" => "The most text the model can take in at once (the conversation and any files), in tokens. Longer contexts need more memory.",
        "Precision" | "Quantisation" => {
            "How each weight is stored. 16-bit (F16, BF16) is full precision; quantised formats such as Q8_0 or Q4_K_M use fewer bits, making the file smaller and faster to run, at some cost to quality that depends on the model."
        }
        "Layers" | "Transformer layers" => {
            "The model is a stack of similar layers. Each one reads the text's representation from the layer before and refines it; more layers usually means more capable and more memory."
        }
        "Hidden size" => "How many numbers represent each token inside the model, between layers.",
        "Attention heads" | "Attention" => {
            "Attention lets each token draw on the tokens before it. Each head does this separately, looking for different kinds of connection."
        }
        "Key/value heads" => "Heads that store what attention looks back at. Fewer of them than attention heads (grouped-query attention) makes the KV cache smaller.",
        "KV cache" | "Cache" => "Memory that holds what the model has already read, so it doesn't read it again for every new token. It grows with the context length.",
        "Experts" => "A mixture-of-experts model has several feed-forward blocks per layer and uses only a few for each token, so it runs faster than its size suggests but still needs memory for all of them.",
        "Feed-forward size" => "The width of each layer's feed-forward block, where most of the model's knowledge is thought to be stored.",
        "Embeddings" => "The table that turns each token into numbers at the start, and the numbers themselves. Embedding models output these numbers to compare meaning.",
        "Chat template" => "How a conversation is laid out for the model (who said what), as the model was trained to see it.",
        "Architecture" => "The model's design, such as llama, qwen3 or gemma3. Apps need to support an architecture to run it.",
        "Temperature" => "How adventurous token choice is. 0 always takes the most likely token; higher values give unlikelier tokens more of a chance.",
        "Sampling" => "Picking the next token from the model's probabilities, after settings such as temperature, Top-K, Top-P and Min-P reshape them.",
        "Steering" | "Activation steering" => {
            "Nudging the model's internal numbers in one direction while it writes. It changes nothing in the model file, and its effect is experimental."
        }
        "GPU offloading" => "Running some or all of a model's layers on the graphics chip, which is usually much faster, if it has enough memory.",
        "Licence" => "The terms the publisher released the model under, as the model or its files state it. Not checked by Filescape.",
        _ => "",
    }
}
