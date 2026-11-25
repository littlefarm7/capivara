//! Arquitetura do Whisper (pesos do HuggingFace) com decodificação incremental:
//! o decoder guarda K/V da auto-atenção, então cada token novo custa O(1) camadas
//! em vez de reprocessar a frase inteira (o modelo do candle só cacheia a atenção cruzada).
//! Baseado em candle-transformers (MIT/Apache-2.0) e openai/whisper (MIT).

use candle_core::{Device, IndexOp, Result, Tensor};
use candle_nn::{Conv1d, Conv1dConfig, Embedding, LayerNorm, Linear, Module, VarBuilder, embedding, linear, linear_no_bias};
pub use candle_transformers::models::whisper::Config;

fn layer_norm(size: usize, vb: VarBuilder) -> Result<LayerNorm> {
    Ok(LayerNorm::new(vb.get(size, "weight")?, vb.get(size, "bias")?, 1e-5))
}

fn conv1d(cin: usize, cout: usize, stride: usize, vb: VarBuilder) -> Result<Conv1d> {
    let cfg = Conv1dConfig { padding: 1, stride, groups: 1, dilation: 1, cudnn_fwd_algo: None };
    Ok(Conv1d::new(vb.get((cout, cin, 3), "weight")?, Some(vb.get(cout, "bias")?), cfg))
}

struct Attn {
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    n_head: usize,
    cache: Option<(Tensor, Tensor)>,
}

impl Attn {
    fn load(n: usize, n_head: usize, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            q: linear(n, n, vb.pp("q_proj"))?,
            k: linear_no_bias(n, n, vb.pp("k_proj"))?,
            v: linear(n, n, vb.pp("v_proj"))?,
            o: linear(n, n, vb.pp("out_proj"))?,
            n_head,
            cache: None,
        })
    }

    fn heads(&self, x: &Tensor) -> Result<Tensor> {
        let (b, t, c) = x.dims3()?;
        x.reshape((b, t, self.n_head, c / self.n_head))?.transpose(1, 2)?.contiguous()
    }

    fn attend(&self, q: &Tensor, k: &Tensor, v: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let (_, _, c) = q.dims3()?;
        let scale = ((c / self.n_head) as f64).powf(-0.25);
        let q = (self.heads(q)? * scale)?;
        let k = (self.heads(k)? * scale)?;
        let v = self.heads(v)?;
        let mut qk = q.matmul(&k.t()?)?;
        if let Some(m) = mask {
            qk = qk.broadcast_add(m)?;
        }
        let w = candle_nn::ops::softmax_last_dim(&qk)?;
        let wv = w.matmul(&v)?.transpose(1, 2)?.flatten_from(2)?;
        self.o.forward(&wv)
    }

    /// Auto-atenção completa (encoder).
    fn full(&self, x: &Tensor) -> Result<Tensor> {
        self.attend(&self.q.forward(x)?, &self.k.forward(x)?, &self.v.forward(x)?, None)
    }

    /// Auto-atenção do decoder com cache incremental.
    fn causal(&mut self, x: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let q = self.q.forward(x)?;
        let k = self.k.forward(x)?;
        let v = self.v.forward(x)?;
        let (k, v) = match &self.cache {
            Some((pk, pv)) => (Tensor::cat(&[pk, &k], 1)?, Tensor::cat(&[pv, &v], 1)?),
            None => (k, v),
        };
        self.cache = Some((k.clone(), v.clone()));
        self.attend(&q, &k, &v, mask)
    }

    /// Atenção cruzada (K/V do áudio calculados uma vez por janela).
    fn cross(&mut self, x: &Tensor, xa: &Tensor) -> Result<Tensor> {
        if self.cache.is_none() {
            self.cache = Some((self.k.forward(xa)?, self.v.forward(xa)?));
        }
        let (k, v) = self.cache.as_ref().unwrap();
        self.attend(&self.q.forward(x)?, k, v, None)
    }
}

struct Mlp {
    fc1: Linear,
    fc2: Linear,
    ln: LayerNorm,
}

impl Mlp {
    fn load(n: usize, vb: &VarBuilder) -> Result<Self> {
        Ok(Self {
            fc1: linear(n, n * 4, vb.pp("fc1"))?,
            fc2: linear(n * 4, n, vb.pp("fc2"))?,
            ln: layer_norm(n, vb.pp("final_layer_norm"))?,
        })
    }
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        x + self.fc2.forward(&self.fc1.forward(&self.ln.forward(x)?)?.gelu_erf()?)?
    }
}

struct EncBlock {
    attn: Attn,
    ln: LayerNorm,
    mlp: Mlp,
}

struct DecBlock {
    attn: Attn,
    ln: LayerNorm,
    xattn: Attn,
    xln: LayerNorm,
    mlp: Mlp,
}

pub struct Encoder {
    conv1: Conv1d,
    conv2: Conv1d,
    pos: Tensor,
    blocks: Vec<EncBlock>,
    ln_post: LayerNorm,
}

fn sinusoids(length: usize, channels: usize, dev: &Device) -> Result<Tensor> {
    let inc = 10000f32.ln() / (channels / 2 - 1) as f32;
    let inv: Vec<f32> = (0..channels / 2).map(|i| (i as f32 * -inc).exp()).collect();
    let inv = Tensor::new(inv.as_slice(), dev)?.unsqueeze(0)?;
    let t = Tensor::arange(0, length as u32, dev)?.to_dtype(candle_core::DType::F32)?.unsqueeze(1)?;
    let sh = (length, channels / 2);
    let st = (t.broadcast_as(sh)? * inv.broadcast_as(sh)?)?;
    Tensor::cat(&[st.sin()?, st.cos()?], 1)
}

impl Encoder {
    fn load(vb: VarBuilder, c: &Config) -> Result<Self> {
        let n = c.d_model;
        let blocks = (0..c.encoder_layers)
            .map(|i| {
                let vb = vb.pp(format!("layers.{i}"));
                Ok(EncBlock {
                    attn: Attn::load(n, c.encoder_attention_heads, vb.pp("self_attn"))?,
                    ln: layer_norm(n, vb.pp("self_attn_layer_norm"))?,
                    mlp: Mlp::load(n, &vb)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            conv1: conv1d(c.num_mel_bins, n, 1, vb.pp("conv1"))?,
            conv2: conv1d(n, n, 2, vb.pp("conv2"))?,
            pos: sinusoids(c.max_source_positions, n, vb.device())?,
            blocks,
            ln_post: layer_norm(n, vb.pp("layer_norm"))?,
        })
    }

    pub fn forward(&self, mel: &Tensor) -> Result<Tensor> {
        let x = self.conv1.forward(mel)?.gelu_erf()?;
        let x = self.conv2.forward(&x)?.gelu_erf()?.transpose(1, 2)?;
        let (_, t, _) = x.dims3()?;
        let mut x = x.broadcast_add(&self.pos.narrow(0, 0, t)?)?;
        for b in &self.blocks {
            x = (&x + b.attn.full(&b.ln.forward(&x)?)?)?;
            x = b.mlp.forward(&x)?;
        }
        self.ln_post.forward(&x)
    }
}

pub struct Decoder {
    tok: Embedding,
    pos: Tensor,
    blocks: Vec<DecBlock>,
    ln: LayerNorm,
    offset: usize,
    device: Device,
}

impl Decoder {
    fn load(vb: VarBuilder, c: &Config) -> Result<Self> {
        let n = c.d_model;
        let blocks = (0..c.decoder_layers)
            .map(|i| {
                let vb = vb.pp(format!("layers.{i}"));
                Ok(DecBlock {
                    attn: Attn::load(n, c.decoder_attention_heads, vb.pp("self_attn"))?,
                    ln: layer_norm(n, vb.pp("self_attn_layer_norm"))?,
                    xattn: Attn::load(n, c.decoder_attention_heads, vb.pp("encoder_attn"))?,
                    xln: layer_norm(n, vb.pp("encoder_attn_layer_norm"))?,
                    mlp: Mlp::load(n, &vb)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            tok: embedding(c.vocab_size, n, vb.pp("embed_tokens"))?,
            pos: vb.get((c.max_target_positions, n), "embed_positions.weight")?,
            blocks,
            ln: layer_norm(n, vb.pp("layer_norm"))?,
            offset: 0,
            device: vb.device().clone(),
        })
    }

    /// Começa uma nova sequência (limpa todos os caches).
    pub fn reset(&mut self) {
        self.offset = 0;
        for b in &mut self.blocks {
            b.attn.cache = None;
            b.xattn.cache = None;
        }
    }

    /// Alimenta só os tokens novos; devolve os estados ocultos deles (1, n, d).
    pub fn forward(&mut self, tokens: &[u32], xa: &Tensor) -> Result<Tensor> {
        let n = tokens.len();
        let o = self.offset;
        let t = Tensor::new(tokens, &self.device)?.unsqueeze(0)?;
        let mut x = self.tok.forward(&t)?.broadcast_add(&self.pos.narrow(0, o, n)?)?;
        let mask = if n > 1 {
            let m: Vec<f32> = (0..n)
                .flat_map(|i| (0..o + n).map(move |j| if j > o + i { f32::NEG_INFINITY } else { 0.0 }))
                .collect();
            Some(Tensor::from_vec(m, (n, o + n), &self.device)?)
        } else {
            None
        };
        for b in &mut self.blocks {
            x = (&x + b.attn.causal(&b.ln.forward(&x)?, mask.as_ref())?)?;
            x = (&x + b.xattn.cross(&b.xln.forward(&x)?, xa)?)?;
            x = b.mlp.forward(&x)?;
        }
        self.offset += n;
        self.ln.forward(&x)
    }

    /// Logits (vocabulário) da posição `pos` dos estados ocultos `h`.
    pub fn logits(&self, h: &Tensor, pos: usize) -> Result<Vec<f32>> {
        let row = h.i((0, pos..pos + 1))?; // (1, d)
        row.matmul(&self.tok.embeddings().t()?)?.i(0)?.to_vec1::<f32>()
    }
}

pub struct Model {
    pub encoder: Encoder,
    pub decoder: Decoder,
    pub config: Config,
}

impl Model {
    pub fn load(vb: &VarBuilder, config: Config) -> Result<Self> {
        Ok(Self {
            encoder: Encoder::load(vb.pp("model.encoder"), &config)?,
            decoder: Decoder::load(vb.pp("model.decoder"), &config)?,
            config,
        })
    }
}
