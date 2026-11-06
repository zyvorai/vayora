<?php
/**
 * Plugin Name: Zyvor Payments
 * Description: Take payments via Zyvor (intents / payment links). Scaffold — not production-ready.
 * Version: 0.2.0
 */

if (!defined('ABSPATH')) {
    exit;
}

add_action('plugins_loaded', function () {
    if (!class_exists('WC_Payment_Gateway')) {
        return;
    }

    class WC_Gateway_Zyvor extends WC_Payment_Gateway {
        public function __construct() {
            $this->id = 'zyvor';
            $this->method_title = 'Zyvor';
            $this->method_description = 'Pay via Zyvor Checkout';
            $this->has_fields = false;
            $this->init_form_fields();
            $this->init_settings();
            $this->title = $this->get_option('title', 'Zyvor');
            add_action('woocommerce_update_options_payment_gateways_' . $this->id, [$this, 'process_admin_options']);
        }

        public function init_form_fields() {
            $this->form_fields = [
                'enabled' => [
                    'title' => 'Enable',
                    'type' => 'checkbox',
                    'default' => 'no',
                ],
                'api_url' => [
                    'title' => 'Zyvor API URL',
                    'type' => 'text',
                    'default' => 'https://api.example.com',
                ],
                'api_key' => [
                    'title' => 'API key',
                    'type' => 'password',
                ],
                'checkout_url' => [
                    'title' => 'Checkout public URL',
                    'type' => 'text',
                ],
                'webhook_secret' => [
                    'title' => 'Webhook signing secret',
                    'type' => 'password',
                    'description' => 'Point Zyvor webhooks at /wc-api/zyvor_webhook',
                ],
            ];
        }

        public function process_payment($order_id) {
            $order = wc_get_order($order_id);
            $amount_minor = (int) round($order->get_total() * 100);
            $api = rtrim($this->get_option('api_url'), '/');
            $res = wp_remote_post($api . '/v1/payment_links', [
                'headers' => [
                    'Authorization' => 'Bearer ' . $this->get_option('api_key'),
                    'Content-Type' => 'application/json',
                ],
                'body' => wp_json_encode([
                    'amount_minor' => $amount_minor,
                    'currency' => 'INR',
                    'external_ref' => 'woo-' . $order_id,
                    'description' => 'Order ' . $order->get_order_number(),
                ]),
                'timeout' => 30,
            ]);
            if (is_wp_error($res)) {
                wc_add_notice('Payment could not be started.', 'error');
                return ['result' => 'fail'];
            }
            $body = json_decode(wp_remote_retrieve_body($res), true);
            $checkout = rtrim($this->get_option('checkout_url'), '/');
            $url = isset($body['id']) ? $checkout . '/?link=' . rawurlencode($body['id']) : '';
            if (!$url) {
                wc_add_notice('Payment could not be started.', 'error');
                return ['result' => 'fail'];
            }
            return [
                'result' => 'success',
                'redirect' => $url,
            ];
        }
    }

    add_filter('woocommerce_payment_gateways', function ($methods) {
        $methods[] = 'WC_Gateway_Zyvor';
        return $methods;
    });
});

/** Mark order paid when Zyvor sends payment.captured (HMAC Zyvor-Signature). */
add_action('woocommerce_api_zyvor_webhook', function () {
    $gateways = WC()->payment_gateways()->payment_gateways();
    $gw = $gateways['zyvor'] ?? null;
    if (!$gw) {
        status_header(503);
        exit;
    }
    $raw = file_get_contents('php://input');
    $header = isset($_SERVER['HTTP_ZYVOR_SIGNATURE']) ? $_SERVER['HTTP_ZYVOR_SIGNATURE'] : '';
    $secret = $gw->get_option('webhook_secret');
    if (!$secret || !zyvor_verify_signature($secret, $header, $raw)) {
        status_header(401);
        echo 'invalid signature';
        exit;
    }
    $payload = json_decode($raw, true);
    $type = is_array($payload) ? ($payload['type'] ?? $payload['event'] ?? '') : '';
    if ($type !== 'payment.captured') {
        status_header(204);
        exit;
    }
    $data = is_array($payload) ? ($payload['data'] ?? $payload) : [];
    $external = is_array($data) ? (string) ($data['external_ref'] ?? '') : '';
    if (!str_starts_with($external, 'woo-')) {
        status_header(204);
        exit;
    }
    $order_id = (int) substr($external, 4);
    $order = wc_get_order($order_id);
    if (!$order) {
        status_header(404);
        exit;
    }
    if (!$order->is_paid()) {
        $order->payment_complete();
        $order->add_order_note('Zyvor payment.captured');
    }
    status_header(204);
    exit;
});

function zyvor_verify_signature($secret, $header, $rawBody) {
    $parts = [];
    foreach (explode(',', (string) $header) as $piece) {
        $kv = explode('=', trim($piece), 2);
        if (count($kv) === 2) {
            $parts[$kv[0]] = $kv[1];
        }
    }
    if (!isset($parts['t'], $parts['v1'])) {
        return false;
    }
    $t = (int) $parts['t'];
    if (abs(time() - $t) > 300) {
        return false;
    }
    $expected = hash_hmac('sha256', $t . '.' . $rawBody, $secret);
    return hash_equals($expected, $parts['v1']);
}
