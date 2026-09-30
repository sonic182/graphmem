defmodule DemoWeb.CoreComponents do
  use Phoenix.Component
  alias Phoenix.LiveView.JS

  attr :kind, :atom, default: :info
  slot :inner_block, required: true

  def flash(assigns) do
    ~H"""
    <div class="flash">
      <.icon name="hero-x-mark" />
      <%= render_slot(@inner_block) %>
    </div>
    """
  end

  def button(%{disabled: true} = assigns), do: ~H"<button disabled><%= @label %></button>"

  def button(assigns) do
    ~H"""
    <button><.icon name="hero-check" />{@label}</button>
    """
  end

  defp icon_class(name) when is_binary(name), do: "icon #{name}"

  defmacro __using__(_opts) do
    quote do
      import DemoWeb.CoreComponents
    end
  end

  defmodule Helpers do
    def hide(js \\ %JS{}, selector) do
      JS.hide(js, to: selector)
    end

    def version, do: "1"
  end
end

require Logger
