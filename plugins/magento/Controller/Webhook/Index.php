<?php
declare(strict_types=1);

namespace Zyvor\Payments\Controller\Webhook;

use Magento\Framework\App\Action\HttpPostActionInterface;
use Magento\Framework\App\CsrfAwareActionInterface;
use Magento\Framework\App\Request\InvalidRequestException;
use Magento\Framework\App\RequestInterface;
use Magento\Framework\App\Config\ScopeConfigInterface;
use Magento\Framework\Controller\Result\RawFactory;
use Magento\Sales\Api\OrderRepositoryInterface;
use Magento\Sales\Model\Order;
use Magento\Store\Model\ScopeInterface;
use Magento\Framework\Api\SearchCriteriaBuilder;

/**
 * Verify Zyvor HMAC and mark the Magento order paid on payment.captured.
 * Match via external_ref magento-{increment_id}.
 */
class Index implements HttpPostActionInterface, CsrfAwareActionInterface
{
    public function __construct(
        private readonly RawFactory $rawFactory,
        private readonly ScopeConfigInterface $scopeConfig,
        private readonly OrderRepositoryInterface $orderRepository,
        private readonly SearchCriteriaBuilder $searchCriteriaBuilder,
        private readonly RequestInterface $request
    ) {
    }

    public function createCsrfValidationException(RequestInterface $request): ?InvalidRequestException
    {
        return null;
    }

    public function validateForCsrf(RequestInterface $request): ?bool
    {
        return true;
    }

    public function execute()
    {
        $result = $this->rawFactory->create();
        $raw = (string) $this->request->getContent();
        $header = (string) $this->request->getHeader('Zyvor-Signature');
        $secret = (string) $this->scopeConfig->getValue('payment/zyvor/webhook_secret', ScopeInterface::SCOPE_STORE);
        if ($secret === '' || !$this->verifySignature($secret, $header, $raw)) {
            $result->setHttpResponseCode(401);
            $result->setContents('invalid signature');
            return $result;
        }

        $payload = json_decode($raw, true);
        $type = is_array($payload) ? ($payload['type'] ?? $payload['event'] ?? '') : '';
        if ($type !== 'payment.captured') {
            $result->setHttpResponseCode(204);
            return $result;
        }

        $data = is_array($payload) ? ($payload['data'] ?? $payload) : [];
        $external = is_array($data) ? (string) ($data['external_ref'] ?? '') : '';
        if (!str_starts_with($external, 'magento-')) {
            $result->setHttpResponseCode(204);
            return $result;
        }
        $incrementId = substr($external, strlen('magento-'));
        $criteria = $this->searchCriteriaBuilder
            ->addFilter('increment_id', $incrementId, 'eq')
            ->create();
        $list = $this->orderRepository->getList($criteria)->getItems();
        $order = $list ? reset($list) : null;
        if (!$order instanceof Order) {
            $result->setHttpResponseCode(404);
            $result->setContents('order not found');
            return $result;
        }
        if ($order->getState() !== Order::STATE_COMPLETE && $order->getState() !== Order::STATE_PROCESSING) {
            $order->setState(Order::STATE_PROCESSING)->setStatus(Order::STATE_PROCESSING);
            $order->addCommentToStatusHistory('Zyvor payment.captured');
            $this->orderRepository->save($order);
        }
        $result->setHttpResponseCode(204);
        return $result;
    }

    private function verifySignature(string $secret, string $header, string $rawBody): bool
    {
        // Header: t=<unix>,v1=<hex hmac-sha256("{t}.{body}")>
        $parts = [];
        foreach (explode(',', $header) as $piece) {
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
}
