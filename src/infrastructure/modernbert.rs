use candle_core::{D, DType, Module, Result, Tensor};
use candle_nn::{
    Activation, Embedding, LayerNorm, Linear, VarBuilder, embedding, layer_norm_no_bias,
    linear_no_bias, ops::softmax_last_dim, rotary_emb::rope,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    vocab_size: usize,
    hidden_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    intermediate_size: usize,
    max_position_embeddings: usize,
    #[serde(default = "default_norm_eps")]
    norm_eps: f64,
    global_attn_every_n_layers: usize,
    global_rope_theta: f64,
    local_attention: usize,
    local_rope_theta: f64,
    #[serde(default)]
    hidden_activation: Activation,
}

fn default_norm_eps() -> f64 {
    1e-5
}

struct Layer {
    attn_norm: Option<LayerNorm>,
    wqkv: Linear,
    attn_out: Linear,
    mlp_norm: LayerNorm,
    wi: Linear,
    mlp_out: Linear,
    cos: Tensor,
    sin: Tensor,
    local: bool,
}

pub struct ModernBert {
    embeddings: Embedding,
    embeddings_norm: LayerNorm,
    layers: Vec<Layer>,
    final_norm: LayerNorm,
    heads: usize,
    window: usize,
    activation: Activation,
    dtype: DType,
}

impl ModernBert {
    pub fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let hidden = config.hidden_size;
        let eps = config.norm_eps;
        let head = hidden / config.num_attention_heads;
        let global = rotary(config.global_rope_theta, head, config, &vb)?;
        let local = rotary(config.local_rope_theta, head, config, &vb)?;
        let layers = (0..config.num_hidden_layers)
            .map(|index| {
                let vb = vb.pp(format!("layers.{index}"));
                let is_local = index % config.global_attn_every_n_layers != 0;
                let (cos, sin) = if is_local { &local } else { &global };
                Ok(Layer {
                    attn_norm: layer_norm_no_bias(hidden, eps, vb.pp("attn_norm")).ok(),
                    wqkv: linear_no_bias(hidden, hidden * 3, vb.pp("attn.Wqkv"))?,
                    attn_out: linear_no_bias(hidden, hidden, vb.pp("attn.Wo"))?,
                    mlp_norm: layer_norm_no_bias(hidden, eps, vb.pp("mlp_norm"))?,
                    wi: linear_no_bias(hidden, config.intermediate_size * 2, vb.pp("mlp.Wi"))?,
                    mlp_out: linear_no_bias(config.intermediate_size, hidden, vb.pp("mlp.Wo"))?,
                    cos: cos.clone(),
                    sin: sin.clone(),
                    local: is_local,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            embeddings: embedding(
                config.vocab_size,
                hidden,
                vb.pp("embeddings.tok_embeddings").to_dtype(DType::BF16),
            )?,
            embeddings_norm: layer_norm_no_bias(hidden, eps, vb.pp("embeddings.norm"))?,
            layers,
            final_norm: layer_norm_no_bias(hidden, eps, vb.pp("final_norm"))?,
            heads: config.num_attention_heads,
            window: config.local_attention / 2,
            activation: config.hidden_activation,
            dtype: vb.dtype(),
        })
    }

    pub fn forward(&self, ids: &Tensor, attention: Option<&Tensor>) -> Result<Tensor> {
        let (batch, length) = ids.dims2()?;
        let padding = match attention {
            Some(attention) => Some(
                ((1.0 - attention.to_dtype(DType::F32)?)? * -1e30)?
                    .to_dtype(self.dtype)?
                    .reshape((batch, 1, 1, length))?,
            ),
            None => None,
        };
        let local = if length > self.window + 1 {
            let window = self.window as isize;
            let mask: Vec<f32> = (0..length as isize)
                .flat_map(|i| {
                    (0..length as isize).map(move |j| {
                        if (i - j).abs() > window {
                            f32::NEG_INFINITY
                        } else {
                            0.0
                        }
                    })
                })
                .collect();
            let mask =
                Tensor::from_vec(mask, (length, length), ids.device())?.to_dtype(self.dtype)?;
            Some(match &padding {
                Some(padding) => padding.broadcast_add(&mask)?,
                None => mask,
            })
        } else {
            padding.clone()
        };
        let mut xs = ids
            .apply(&self.embeddings)?
            .to_dtype(self.dtype)?
            .apply(&self.embeddings_norm)?;
        for layer in &self.layers {
            let mask = if layer.local { &local } else { &padding };
            xs = self.layer_forward(layer, &xs, mask.as_ref())?;
        }
        xs.apply(&self.final_norm)
    }

    fn layer_forward(&self, layer: &Layer, xs: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let (batch, length, hidden) = xs.dims3()?;
        let head = hidden / self.heads;
        let normed = match &layer.attn_norm {
            Some(norm) => xs.apply(norm)?,
            None => xs.clone(),
        };
        let qkv = normed
            .apply(&layer.wqkv)?
            .reshape((batch, length, 3, self.heads, head))?
            .permute((2, 0, 3, 1, 4))?;
        let q = rope(&qkv.get(0)?.contiguous()?, &layer.cos, &layer.sin)?;
        let k = rope(&qkv.get(1)?.contiguous()?, &layer.cos, &layer.sin)?;
        let v = qkv.get(2)?.contiguous()?;
        let mut scores = (q.matmul(&k.t()?)? * (head as f64).powf(-0.5))?;
        if let Some(mask) = mask {
            scores = scores.broadcast_add(mask)?;
        }
        let attended = softmax_last_dim(&scores)?
            .matmul(&v)?
            .transpose(1, 2)?
            .reshape((batch, length, hidden))?
            .apply(&layer.attn_out)?;
        let xs = (xs + attended)?;
        let gate = xs
            .apply(&layer.mlp_norm)?
            .apply(&layer.wi)?
            .chunk(2, D::Minus1)?;
        let mlp = (self.activation.forward(&gate[0])? * &gate[1])?.apply(&layer.mlp_out)?;
        xs + mlp
    }
}

fn rotary(theta: f64, head: usize, config: &Config, vb: &VarBuilder) -> Result<(Tensor, Tensor)> {
    let device = vb.device();
    let inv_freq: Vec<f32> = (0..head)
        .step_by(2)
        .map(|i| (1.0 / theta.powf(i as f64 / head as f64)) as f32)
        .collect();
    let inv_freq = Tensor::from_vec(inv_freq, (1, head / 2), device)?;
    let positions = config.max_position_embeddings;
    let freqs = Tensor::arange(0u32, positions as u32, device)?
        .to_dtype(DType::F32)?
        .reshape((positions, 1))?
        .matmul(&inv_freq)?;
    Ok((
        freqs.cos()?.to_dtype(vb.dtype())?,
        freqs.sin()?.to_dtype(vb.dtype())?,
    ))
}
