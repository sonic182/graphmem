use candle_core::{D, DType, Module, Result, Tensor, bail};
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
    #[serde(default)]
    attention_bias: bool,
    #[serde(default)]
    mlp_bias: bool,
    #[serde(default)]
    norm_bias: bool,
    classifier_pooling: String,
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

impl Config {
    fn validate(&self) -> Result<()> {
        if self.attention_bias || self.mlp_bias || self.norm_bias {
            bail!("modernbert checkpoints with attention, mlp, or norm biases are not supported");
        }
        if self.classifier_pooling != "cls" {
            bail!("modernbert checkpoints must use CLS pooling");
        }
        if self.num_attention_heads == 0
            || self.hidden_size == 0
            || !self.hidden_size.is_multiple_of(self.num_attention_heads)
            || self.global_attn_every_n_layers == 0
        {
            bail!(
                "invalid modernbert config: hidden_size {} must split evenly across {} attention \
                 heads and global_attn_every_n_layers must be non-zero",
                self.hidden_size,
                self.num_attention_heads
            );
        }
        Ok(())
    }
}

impl ModernBert {
    pub fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        config.validate()?;
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
                    attn_norm: (index != 0)
                        .then(|| layer_norm_no_bias(hidden, eps, vb.pp("attn_norm")))
                        .transpose()?,
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
                vb.pp("embeddings.tok_embeddings"),
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
            let positions = Tensor::arange(0f32, length as f32, ids.device())?;
            let distance = positions
                .unsqueeze(1)?
                .broadcast_sub(&positions.unsqueeze(0)?)?
                .abs()?;
            let mask = (distance.gt(self.window as f32)?.to_dtype(DType::F32)? * -1e30)?
                .to_dtype(self.dtype)?;
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

#[cfg(test)]
mod tests {
    use super::Config;

    fn config() -> Config {
        serde_json::from_str(
            r#"{
                "vocab_size": 100,
                "hidden_size": 384,
                "num_hidden_layers": 12,
                "num_attention_heads": 12,
                "intermediate_size": 1536,
                "max_position_embeddings": 2048,
                "global_attn_every_n_layers": 3,
                "global_rope_theta": 150000.0,
                "local_attention": 128,
                "local_rope_theta": 160000.0,
                "classifier_pooling": "cls"
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn accepts_only_supported_pooling_and_bias_configuration() {
        assert!(config().validate().is_ok());
        let mut mean_pooling = config();
        mean_pooling.classifier_pooling = "mean".to_owned();
        assert!(mean_pooling.validate().is_err());
        let mut biased = config();
        biased.attention_bias = true;
        assert!(biased.validate().is_err());
    }

    #[test]
    fn rejects_invalid_attention_dimensions_without_panicking() {
        let mut zero_heads = config();
        zero_heads.num_attention_heads = 0;
        assert!(zero_heads.validate().is_err());
        let mut uneven_heads = config();
        uneven_heads.hidden_size = 385;
        assert!(uneven_heads.validate().is_err());
        let mut zero_spacing = config();
        zero_spacing.global_attn_every_n_layers = 0;
        assert!(zero_spacing.validate().is_err());
    }
}
