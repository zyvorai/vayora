<?php
declare(strict_types=1);

namespace Zyvor\Payments\Controller\Checkout;

use Magento\Checkout\Model\Session as CheckoutSession;
use Magento\Framework\App\Action\HttpGetActionInterface;
use Magento\Framework\App\Config\ScopeConfigInterface;
use Magento\Framework\Controller\Result\RedirectFactory;
use Magento\Framework\Message\ManagerInterface;
use Magento\Store\Model\ScopeInterface;

/**
 * After place-order: create a Zyvor payment link and send the shopper to Checkout.
 */
class Redirect implements HttpGetActionInterface
{
    public function __construct(
        private readonly CheckoutSession $checkoutSession,
        private readonly ScopeConfigInterface $scopeConfig,
        private readonly RedirectFactory $redirectFactory,
        private readonly ManagerInterface $messageManager
    ) {
    }

    public function execute()
    {
        $result = $this->redirectFactory->create();
        $order = $this->checkoutSession->getLastRealOrder();
        if (!$order || !$order->getId()) {
            $this->messageManager->addErrorMessage(__('Order not found.'));
            return $result->setPath('checkout/cart');
        }

        $api = rtrim((string) $this->scopeConfig->getValue('payment/zyvor/api_url', ScopeInterface::SCOPE_STORE), '/');
        $key = (string) $this->scopeConfig->getValue('payment/zyvor/api_key', ScopeInterface::SCOPE_STORE);
        $checkout = rtrim((string) $this->scopeConfig->getValue('payment/zyvor/checkout_url', ScopeInterface::SCOPE_STORE), '/');
        $amountMinor = (int) round(((float) $order->getGrandTotal()) * 100);

        $payload = json_encode([
            'amount_minor' => $amountMinor,
            'currency' => 'INR',
            'external_ref' => 'magento-' . $order->getIncrementId(),
            'description' => 'Order ' . $order->getIncrementId(),
        ], JSON_THROW_ON_ERROR);

        $ch = curl_init($api . '/v1/payment_links');
        curl_setopt_array($ch, [
            CURLOPT_POST => true,
            CURLOPT_HTTPHEADER => [
                'Authorization: Bearer ' . $key,
                'Content-Type: application/json',
                'Accept: application/json',
            ],
            CURLOPT_POSTFIELDS => $payload,
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_TIMEOUT => 30,
        ]);
        $body = curl_exec($ch);
        $code = (int) curl_getinfo($ch, CURLINFO_HTTP_CODE);
        curl_close($ch);

        $data = is_string($body) ? json_decode($body, true) : null;
        $linkId = is_array($data) ? ($data['id'] ?? null) : null;
        if ($code < 200 || $code >= 300 || !$linkId) {
            $this->messageManager->addErrorMessage(__('Payment could not be started.'));
            return $result->setPath('checkout/cart');
        }

        return $result->setUrl($checkout . '/?link=' . rawurlencode((string) $linkId));
    }
}
